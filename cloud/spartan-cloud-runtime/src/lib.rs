//! The container-isolation layer for Spartan Cloud: a `ContainerRuntime`
//! trait (the swappable seam) + a real `DockerRuntime` driver (bollard) that
//! applies per-tenant resource caps and selects the OCI runtime.
//!
//! **Isolation is a first-class, honest field, not an assumption.** Running
//! *other people's* build/test code is the highest security bar in this repo,
//! so `DockerRuntime` carries an explicit `isolation_verified` flag that the
//! control plane surfaces. It must be set `true` ONLY when the operator has
//! confirmed, in the actual deployment, that the chosen OCI runtime really
//! isolates and enforces caps.
//!
//! **Real gVisor go/no-go result (this repo's own spike):** `runsc` (gVisor)
//! installs and registers as a Docker runtime here, but its sandbox startup
//! **hangs** inside this already-nested container (no `/dev/kvm`; restricted
//! ptrace/systrap), regardless of platform. So `runc` is the *verified*
//! baseline (confirmed to run containers and enforce memory/cpu/pids caps),
//! and `runsc` is *selectable but unverified in this environment* — a real
//! target deployment on KVM-capable hardware must re-verify it before running
//! untrusted tenant code. Firecracker/Fly-Machines is a documented future
//! driver behind this same trait.
//!
//! Deliberate, load-bearing tenant-separation choices in `DockerRuntime`:
//! per-tenant resource caps (memory/nano_cpus/pids), **no host bind-mounts
//! ever** (a fresh anonymous scratch volume is removed on teardown), and no
//! network in this MVP (`network_mode: none` — the per-tenant network +
//! allowlisted-egress-proxy design is deferred, and "no network" is the
//! safest default until it lands).

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogOutput, RemoveContainerOptions,
    StartContainerOptions, StatsOptions, StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use bollard::image::CreateImageOptions;
use bollard::models::{ContainerStateStatusEnum, HostConfig};
use bollard::Docker;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use spartan_cloud_protocol::{AllocationId, AllocationStatus, UserId};
use spartan_cloud_tenant::PlanLimits;

/// Docker label marking a container as Spartan-Cloud-managed (so listing /
/// reaping never touches unrelated containers on the same daemon).
pub const MANAGED_LABEL: &str = "com.spartan.cloud.managed";
/// Docker label recording the owning user id (tenant scoping + quota counts).
pub const OWNER_LABEL: &str = "com.spartan.cloud.owner";
/// Docker label recording the allocation's hard-kill deadline as a Unix
/// timestamp (seconds). The reaper stops any managed container whose deadline
/// has passed -- the concrete enforcement of `PlanLimits::max_lifetime_secs`
/// and §36.4.7's "uncapped consumption" failure mode. Stored as an absolute
/// instant (not a duration) so it survives the reaper running on any schedule.
pub const DEADLINE_LABEL: &str = "com.spartan.cloud.deadline";

/// Seconds since the Unix epoch, saturating (clock-before-1970 -> 0).
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum RuntimeError {
    /// Any error from the Docker daemon / bollard client.
    Docker(String),
    /// The referenced allocation doesn't exist (or is already gone).
    NotFound,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Docker(e) => write!(f, "container runtime error: {e}"),
            RuntimeError::NotFound => write!(f, "allocation not found"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<bollard::errors::Error> for RuntimeError {
    fn from(e: bollard::errors::Error) -> Self {
        RuntimeError::Docker(e.to_string())
    }
}

/// What to allocate: which user owns it, the image, and the plan's real caps.
#[derive(Debug, Clone)]
pub struct AllocationSpec {
    pub owner: UserId,
    pub image: String,
    pub limits: PlanLimits,
}

/// The result of a one-shot `exec_once`: the command's combined output and its
/// real exit code (`None` if the daemon didn't report one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub output: String,
    pub exit_code: Option<i64>,
}

/// Real commands a live interactive exec session accepts, sent over a plain
/// `tokio::sync::mpsc::UnboundedSender` -- the same real shape
/// `spartan-devcontainer::docker::ExecCommand` already established for its
/// own local dev-container exec sessions (not shared code -- that crate is
/// sync/thread-based and lives in the main workspace; `cloud/` is a
/// deliberately separate workspace, see this crate's own module doc -- but
/// the same real, proven design).
pub enum ExecSessionCommand {
    Input(Vec<u8>),
    Resize(u16, u16),
}

/// A handle to a live interactive exec session (`spawn_interactive_exec`).
/// `write`/`resize` are plain, non-async functions -- a caller (e.g. an axum
/// WebSocket handler pumping frames) can call them from any context without
/// needing to hold an async lock on the session itself.
#[derive(Debug)]
pub struct ExecSessionHandle {
    tx: tokio::sync::mpsc::UnboundedSender<ExecSessionCommand>,
}

impl ExecSessionHandle {
    pub fn write(&self, data: Vec<u8>) -> Result<(), RuntimeError> {
        self.tx
            .send(ExecSessionCommand::Input(data))
            .map_err(|_| RuntimeError::Docker("exec session already closed".to_string()))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), RuntimeError> {
        self.tx
            .send(ExecSessionCommand::Resize(cols, rows))
            .map_err(|_| RuntimeError::Docker("exec session already closed".to_string()))
    }
}

/// A live resource-usage snapshot for one managed container -- the defensive
/// telemetry the admin monitoring dashboard surfaces (the counterpart to the
/// reaper + caps: spotting abuse, not just capping it). Values are best-effort
/// from a single `docker stats` snapshot; a field the daemon didn't report is
/// `None` rather than a fabricated zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerUsage {
    pub id: AllocationId,
    pub owner: UserId,
    pub memory_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub pids: Option<u64>,
}

/// The swappable isolation seam. Every method is tenant-scoped and, on the
/// real driver, resource-capped. A Firecracker/Fly-Machines driver is a
/// future `impl` of exactly this trait.
#[async_trait]
pub trait ContainerRuntime: Send + Sync {
    /// Create + start a capped container for `spec`, returning its id.
    async fn create(&self, spec: &AllocationSpec) -> Result<AllocationId, RuntimeError>;
    /// The current lifecycle status of an allocation.
    async fn status(&self, id: &AllocationId) -> Result<AllocationStatus, RuntimeError>;
    /// Stop + remove an allocation (and its scratch volume). Idempotent.
    async fn stop(&self, id: &AllocationId) -> Result<(), RuntimeError>;
    /// All allocation ids currently owned by `owner`.
    async fn list_owned(&self, owner: &UserId) -> Result<Vec<AllocationId>, RuntimeError>;
    /// How many of `owner`'s allocations are running right now (feeds quota
    /// admission -- the real `current_active` the API's `can_allocate` needs).
    async fn count_active(&self, owner: &UserId) -> Result<u32, RuntimeError>;
    /// Stop + remove every managed allocation (across ALL tenants) whose
    /// deadline is at or before `now_unix`, returning the reaped ids. This is
    /// the hard enforcement of `PlanLimits::max_lifetime_secs`: a container
    /// that outlives its plan's lifetime is killed regardless of tenant
    /// activity. `now_unix` is a parameter (not read from the clock inside)
    /// purely so it's testable without waiting out a real lifetime.
    async fn reap_expired(&self, now_unix: u64) -> Result<Vec<AllocationId>, RuntimeError>;
    /// A live per-container resource snapshot for every managed running
    /// container (all tenants), for the admin monitoring dashboard.
    async fn usage(&self) -> Result<Vec<ContainerUsage>, RuntimeError>;
    /// Run a one-shot command inside `owner`'s allocation `id`, returning its
    /// combined stdout+stderr and exit code. **Owner-scoped**: the allocation
    /// is verified to actually belong to `owner` (via its label) before any
    /// command runs, so one tenant can never exec into another's container --
    /// the same per-tenant isolation invariant every other method enforces.
    /// A missing/foreign allocation is `RuntimeError::NotFound`.
    async fn exec_once(
        &self,
        owner: &UserId,
        id: &AllocationId,
        command: &[String],
    ) -> Result<ExecResult, RuntimeError>;
    /// Spawn a real, interactive `docker exec -it`-equivalent session inside
    /// `owner`'s allocation `id` -- the streaming counterpart to `exec_once`,
    /// for the per-container WebSocket exec session. **Owner-scoped**
    /// identically to `exec_once`: a foreign or unknown allocation is refused
    /// with `NotFound` before anything is spawned. `on_output` is called with
    /// each real output chunk as it arrives; `on_exit` is called exactly once
    /// when the session ends for any reason (the shell exited, the container
    /// stopped, a real Docker error). Boxed closures, not generics -- this
    /// trait is used as `dyn ContainerRuntime`, and a generic method would not
    /// be object-safe.
    async fn spawn_interactive_exec(
        &self,
        owner: &UserId,
        id: &AllocationId,
        cols: u16,
        rows: u16,
        on_output: Box<dyn FnMut(Vec<u8>) + Send>,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<ExecSessionHandle, RuntimeError>;
    /// Whether this runtime's isolation is verified in the current deployment.
    /// `false` is an ops signal: do NOT run untrusted tenant code yet.
    fn isolation_verified(&self) -> bool;
    /// The OCI runtime name in use ("runc", "runsc", ...).
    fn oci_runtime(&self) -> &str;
}

/// The real Docker-backed runtime driver.
pub struct DockerRuntime {
    docker: Docker,
    oci_runtime: String,
    isolation_verified: bool,
}

impl DockerRuntime {
    /// Connect to the local Docker daemon. `oci_runtime` selects the OCI
    /// runtime ("runc" or "runsc"); `isolation_verified` records whether that
    /// runtime's isolation has been confirmed in THIS deployment (see the
    /// module docs -- pass `false` for gVisor until a real target re-verifies
    /// it, since it does not run in this nested sandbox).
    pub fn connect(
        oci_runtime: impl Into<String>,
        isolation_verified: bool,
    ) -> Result<Self, RuntimeError> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self {
            docker,
            oci_runtime: oci_runtime.into(),
            isolation_verified,
        })
    }

    /// Construct from an existing `Docker` handle (tests, custom transports).
    pub fn with_docker(
        docker: Docker,
        oci_runtime: impl Into<String>,
        isolation_verified: bool,
    ) -> Self {
        Self {
            docker,
            oci_runtime: oci_runtime.into(),
            isolation_verified,
        }
    }

    /// Ensure `image` is present locally, pulling it if absent. A real
    /// allocation can't start a container from an image the daemon doesn't
    /// have -- a fresh host (or CI runner) has nothing pre-pulled. Inspect
    /// first so the common already-present case is a single cheap call and no
    /// network hit; only pull on a real 404.
    async fn ensure_image(&self, image: &str) -> Result<(), RuntimeError> {
        match self.docker.inspect_image(image).await {
            Ok(_) => return Ok(()), // already present
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {} // fall through to pull
            Err(e) => return Err(e.into()),
        }
        let options = CreateImageOptions {
            from_image: image.to_string(),
            ..Default::default()
        };
        // create_image yields a progress stream that must be drained to
        // completion; a mid-stream error means the pull failed.
        let mut stream = self.docker.create_image(Some(options), None, None);
        while let Some(item) = stream.next().await {
            item?;
        }
        Ok(())
    }

    /// Verify `id` is a real, currently-managed container owned by `owner`.
    /// Shared by `exec_once` and `spawn_interactive_exec` -- the identical
    /// owner-scoping check both need before running anything inside a
    /// container. A container that doesn't exist, isn't managed, or belongs
    /// to a different tenant is `NotFound` -- deliberately the same error a
    /// wholly unknown allocation gets, so this check itself can never be used
    /// to probe whether a given id belongs to someone else.
    async fn verify_owned(&self, owner: &UserId, id: &AllocationId) -> Result<(), RuntimeError> {
        let inspect = match self.docker.inspect_container(&id.0, None).await {
            Ok(c) => c,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Err(RuntimeError::NotFound),
            Err(e) => return Err(e.into()),
        };
        let owned = inspect
            .config
            .as_ref()
            .and_then(|c| c.labels.as_ref())
            .map(|l| {
                l.get(MANAGED_LABEL).map(String::as_str) == Some("1")
                    && l.get(OWNER_LABEL).map(String::as_str) == Some(owner.0.as_str())
            })
            .unwrap_or(false);
        if owned {
            Ok(())
        } else {
            Err(RuntimeError::NotFound)
        }
    }

    fn host_config(&self, limits: &PlanLimits) -> HostConfig {
        HostConfig {
            // memory_mb -> bytes.
            memory: Some((limits.memory_mb as i64) * 1024 * 1024),
            // cpu_millis -> nano_cpus (1000 millis = 1 core = 1e9 nano_cpus).
            nano_cpus: Some((limits.cpu_millis as i64) * 1_000_000),
            pids_limit: Some(limits.pids_limit as i64),
            runtime: Some(self.oci_runtime.clone()),
            // No network in the MVP -- safest default until per-tenant nets land.
            network_mode: Some("none".to_string()),
            // NEVER any host bind-mounts -- that would cross tenants. Only the
            // container's own writable layer + an anonymous scratch volume
            // (removed on stop via RemoveContainerOptions.v).
            ..Default::default()
        }
    }
}

fn map_status(status: Option<ContainerStateStatusEnum>) -> AllocationStatus {
    match status {
        Some(ContainerStateStatusEnum::CREATED) => AllocationStatus::Provisioning,
        Some(ContainerStateStatusEnum::RUNNING) => AllocationStatus::Running,
        Some(ContainerStateStatusEnum::RESTARTING) => AllocationStatus::Provisioning,
        Some(ContainerStateStatusEnum::REMOVING) => AllocationStatus::Stopping,
        Some(ContainerStateStatusEnum::PAUSED) => AllocationStatus::Running,
        Some(ContainerStateStatusEnum::EXITED) => AllocationStatus::Stopped,
        Some(ContainerStateStatusEnum::DEAD) => AllocationStatus::Failed,
        Some(ContainerStateStatusEnum::EMPTY) | None => AllocationStatus::Provisioning,
    }
}

#[async_trait]
impl ContainerRuntime for DockerRuntime {
    async fn create(&self, spec: &AllocationSpec) -> Result<AllocationId, RuntimeError> {
        // A container can't start from an image the daemon doesn't have.
        self.ensure_image(&spec.image).await?;

        let mut labels = HashMap::new();
        labels.insert(MANAGED_LABEL.to_string(), "1".to_string());
        labels.insert(OWNER_LABEL.to_string(), spec.owner.0.clone());
        // Absolute hard-kill deadline = creation time + the plan's lifetime.
        let deadline = now_unix().saturating_add(spec.limits.max_lifetime_secs);
        labels.insert(DEADLINE_LABEL.to_string(), deadline.to_string());

        let config = Config {
            image: Some(spec.image.clone()),
            labels: Some(labels),
            host_config: Some(self.host_config(&spec.limits)),
            // Keep the workspace container alive so it can be exec'd into. A
            // real workspace image would run its own entrypoint; for a bare
            // base image, an idle sleep stands in.
            cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
            ..Default::default()
        };

        let created = self
            .docker
            .create_container(None::<CreateContainerOptions<String>>, config)
            .await?;
        self.docker
            .start_container(&created.id, None::<StartContainerOptions<String>>)
            .await?;
        Ok(AllocationId(created.id))
    }

    async fn status(&self, id: &AllocationId) -> Result<AllocationStatus, RuntimeError> {
        match self.docker.inspect_container(&id.0, None).await {
            Ok(c) => Ok(map_status(c.state.and_then(|s| s.status))),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Err(RuntimeError::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    async fn stop(&self, id: &AllocationId) -> Result<(), RuntimeError> {
        // Best-effort stop (ignore "already stopped"), then force-remove the
        // container AND its anonymous volumes (`v: true`).
        let _ = self
            .docker
            .stop_container(&id.0, Some(StopContainerOptions { t: 5 }))
            .await;
        match self
            .docker
            .remove_container(
                &id.0,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(()) => Ok(()),
            // Already gone (404) is success for an idempotent stop. A 409
            // "removal already in progress" is also success: something else
            // (e.g. the background reaper racing an explicit user stop, or a
            // concurrent test) is already tearing down this exact container --
            // the desired end state (gone) is reached either way.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404 | 409,
                ..
            }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn list_owned(&self, owner: &UserId) -> Result<Vec<AllocationId>, RuntimeError> {
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![
                format!("{MANAGED_LABEL}=1"),
                format!("{OWNER_LABEL}={}", owner.0),
            ],
        );
        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };
        let summaries = self.docker.list_containers(Some(options)).await?;
        Ok(summaries
            .into_iter()
            .filter_map(|s| s.id.map(AllocationId))
            .collect())
    }

    async fn count_active(&self, owner: &UserId) -> Result<u32, RuntimeError> {
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![
                format!("{MANAGED_LABEL}=1"),
                format!("{OWNER_LABEL}={}", owner.0),
            ],
        );
        // Only running containers count against the concurrency quota.
        filters.insert("status".to_string(), vec!["running".to_string()]);
        let options = ListContainersOptions {
            all: false,
            filters,
            ..Default::default()
        };
        let summaries = self.docker.list_containers(Some(options)).await?;
        Ok(summaries.len() as u32)
    }

    async fn reap_expired(&self, now_unix: u64) -> Result<Vec<AllocationId>, RuntimeError> {
        // Every managed container (any tenant, any state), so an expired
        // container gets reaped whether it's still running or wedged.
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("{MANAGED_LABEL}=1")]);
        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };
        let summaries = self.docker.list_containers(Some(options)).await?;

        let mut reaped = Vec::new();
        for summary in summaries {
            let Some(id) = summary.id else { continue };
            // A managed container with no/unparseable deadline is treated as
            // already expired -- fail safe (kill it) rather than let an
            // unlabelled container run forever and defeat the whole cap.
            let expired = summary
                .labels
                .as_ref()
                .and_then(|l| l.get(DEADLINE_LABEL))
                .and_then(|d| d.parse::<u64>().ok())
                .map(|deadline| now_unix >= deadline)
                .unwrap_or(true);
            if expired {
                let alloc = AllocationId(id);
                self.stop(&alloc).await?;
                reaped.push(alloc);
            }
        }
        Ok(reaped)
    }

    async fn usage(&self) -> Result<Vec<ContainerUsage>, RuntimeError> {
        // All managed, running containers (across tenants).
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("{MANAGED_LABEL}=1")]);
        filters.insert("status".to_string(), vec!["running".to_string()]);
        let options = ListContainersOptions {
            all: false,
            filters,
            ..Default::default()
        };
        let summaries = self.docker.list_containers(Some(options)).await?;

        let mut out = Vec::new();
        for summary in summaries {
            let Some(id) = summary.id else { continue };
            let owner = summary
                .labels
                .as_ref()
                .and_then(|l| l.get(OWNER_LABEL))
                .cloned()
                .unwrap_or_default();

            // One-shot stats snapshot (no streaming). A container that vanished
            // between the list and the stats call is simply skipped, not fatal.
            let mut stream = self.docker.stats(
                &id,
                Some(StatsOptions {
                    stream: false,
                    one_shot: true,
                }),
            );
            let usage = match stream.next().await {
                Some(Ok(s)) => ContainerUsage {
                    id: AllocationId(id),
                    owner: UserId(owner),
                    memory_bytes: s.memory_stats.usage,
                    memory_limit_bytes: s.memory_stats.limit,
                    pids: s.pids_stats.current,
                },
                _ => ContainerUsage {
                    id: AllocationId(id),
                    owner: UserId(owner),
                    memory_bytes: None,
                    memory_limit_bytes: None,
                    pids: None,
                },
            };
            out.push(usage);
        }
        Ok(out)
    }

    async fn exec_once(
        &self,
        owner: &UserId,
        id: &AllocationId,
        command: &[String],
    ) -> Result<ExecResult, RuntimeError> {
        // Owner-scoping is load-bearing: verify this allocation is managed AND
        // owned by `owner` before running anything -- one tenant can never
        // exec into another's container.
        self.verify_owned(owner, id).await?;

        let exec = self
            .docker
            .create_exec(
                &id.0,
                CreateExecOptions {
                    cmd: Some(command.to_vec()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?;

        let mut output = String::new();
        if let StartExecResults::Attached {
            output: mut stream, ..
        } = self.docker.start_exec(&exec.id, None).await?
        {
            while let Some(item) = stream.next().await {
                // Each frame is stdout or stderr bytes; append both (combined),
                // lossy-decoded so a chunk boundary can't error the whole run.
                let bytes = item?.into_bytes();
                output.push_str(&String::from_utf8_lossy(&bytes));
            }
        }

        let exit_code = self.docker.inspect_exec(&exec.id).await?.exit_code;
        Ok(ExecResult { output, exit_code })
    }

    async fn spawn_interactive_exec(
        &self,
        owner: &UserId,
        id: &AllocationId,
        cols: u16,
        rows: u16,
        mut on_output: Box<dyn FnMut(Vec<u8>) + Send>,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<ExecSessionHandle, RuntimeError> {
        // Owner-scoping first, exactly like exec_once -- before any real exec
        // is even created, let alone started.
        self.verify_owned(owner, id).await?;

        let exec = self
            .docker
            .create_exec(
                &id.0,
                CreateExecOptions {
                    cmd: Some(vec!["/bin/sh".to_string()]),
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(true),
                    ..Default::default()
                },
            )
            .await?;

        let start_result = self
            .docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    tty: true,
                    ..Default::default()
                }),
            )
            .await?;
        let (mut output, mut input) = match start_result {
            StartExecResults::Attached { output, input } => (output, input),
            StartExecResults::Detached => {
                return Err(RuntimeError::Docker(
                    "exec started detached unexpectedly (tty was requested)".to_string(),
                ))
            }
        };

        // Real terminal size, set once up front; `ExecSessionHandle::resize`
        // lets a caller (the WS handler, on a real client resize) update it.
        let _ = self
            .docker
            .resize_exec(
                &exec.id,
                ResizeExecOptions {
                    height: rows,
                    width: cols,
                },
            )
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecSessionCommand>();
        let docker = self.docker.clone(); // bollard::Docker is a cheap, Clone handle.
        let exec_id = exec.id.clone();

        // The pump loop owns the real exec streams and runs for the life of
        // the session; the handle returned to the caller only ever talks to
        // it over the channel, never touching the streams directly.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    item = output.next() => {
                        match item {
                            Some(Ok(
                                LogOutput::StdOut { message }
                                | LogOutput::StdErr { message }
                                | LogOutput::Console { message },
                            )) => {
                                on_output(message.to_vec());
                            }
                            Some(Ok(LogOutput::StdIn { .. })) => {}
                            Some(Err(_)) | None => break,
                        }
                    }
                    cmd = rx.recv() => {
                        match cmd {
                            Some(ExecSessionCommand::Input(bytes)) => {
                                if input.write_all(&bytes).await.is_err() {
                                    break;
                                }
                            }
                            Some(ExecSessionCommand::Resize(cols, rows)) => {
                                let _ = docker
                                    .resize_exec(
                                        &exec_id,
                                        ResizeExecOptions { height: rows, width: cols },
                                    )
                                    .await;
                            }
                            None => break,
                        }
                    }
                }
            }
            on_exit();
        });

        Ok(ExecSessionHandle { tx })
    }

    fn isolation_verified(&self) -> bool {
        self.isolation_verified
    }

    fn oci_runtime(&self) -> &str {
        &self.oci_runtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spartan_cloud_protocol::PlanTier;
    use spartan_cloud_tenant::PlanLimits;

    /// Skips (prints a message, passes) if no Docker daemon is reachable --
    /// the same real-external-tool convention `spartan-devcontainer`'s own
    /// `docker_integration.rs` uses.
    async fn docker_or_skip() -> Option<Docker> {
        match Docker::connect_with_local_defaults() {
            Ok(d) => match d.ping().await {
                Ok(_) => Some(d),
                Err(_) => {
                    println!("SKIP: Docker daemon not reachable (ping failed)");
                    None
                }
            },
            Err(_) => {
                println!("SKIP: Docker daemon not reachable (connect failed)");
                None
            }
        }
    }

    #[test]
    fn map_status_covers_the_real_states() {
        assert_eq!(
            map_status(Some(ContainerStateStatusEnum::RUNNING)),
            AllocationStatus::Running
        );
        assert_eq!(
            map_status(Some(ContainerStateStatusEnum::EXITED)),
            AllocationStatus::Stopped
        );
        assert_eq!(
            map_status(Some(ContainerStateStatusEnum::CREATED)),
            AllocationStatus::Provisioning
        );
        assert_eq!(
            map_status(Some(ContainerStateStatusEnum::DEAD)),
            AllocationStatus::Failed
        );
        assert_eq!(map_status(None), AllocationStatus::Provisioning);
    }

    #[test]
    fn host_config_maps_plan_limits_to_real_docker_caps() {
        // Pure check (no daemon): the cap arithmetic is correct.
        let rt = DockerRuntime {
            // A dummy handle is fine -- host_config never touches it.
            docker: Docker::connect_with_local_defaults()
                .unwrap_or_else(|_| panic!("connect builder never does I/O")),
            oci_runtime: "runc".to_string(),
            isolation_verified: true,
        };
        let limits = PlanLimits::for_tier(PlanTier::Free); // 1024 MB, 1000 millis, 256 pids
        let hc = rt.host_config(&limits);
        assert_eq!(hc.memory, Some(1024 * 1024 * 1024));
        assert_eq!(hc.nano_cpus, Some(1_000_000_000)); // 1 core
        assert_eq!(hc.pids_limit, Some(256));
        assert_eq!(hc.runtime.as_deref(), Some("runc"));
        assert_eq!(hc.network_mode.as_deref(), Some("none"));
    }

    #[tokio::test]
    async fn real_lifecycle_create_status_count_stop() {
        let Some(docker) = docker_or_skip().await else {
            return;
        };
        // Verified baseline: runc (gVisor is unverified in this env, see docs).
        let runtime = DockerRuntime::with_docker(docker, "runc", true);
        let owner = UserId(format!("test-owner-{}", std::process::id()));
        let spec = AllocationSpec {
            owner: owner.clone(),
            image: "alpine:latest".to_string(),
            limits: PlanLimits::for_tier(PlanTier::Free),
        };

        let id = runtime
            .create(&spec)
            .await
            .expect("create a real container");

        // It's running, counts against quota, and is listed for its owner.
        assert_eq!(
            runtime.status(&id).await.unwrap(),
            AllocationStatus::Running
        );
        assert_eq!(runtime.count_active(&owner).await.unwrap(), 1);
        assert!(runtime.list_owned(&owner).await.unwrap().contains(&id));

        // The real memory cap is actually applied to the running container.
        let inspect = runtime.docker.inspect_container(&id.0, None).await.unwrap();
        let applied_mem = inspect
            .host_config
            .and_then(|h| h.memory)
            .expect("a real memory cap");
        assert_eq!(
            applied_mem,
            1024 * 1024 * 1024,
            "the plan's memory cap is really enforced"
        );

        // Stop removes it; it's then gone and no longer counted.
        runtime.stop(&id).await.expect("stop the container");
        assert!(matches!(
            runtime.status(&id).await,
            Err(RuntimeError::NotFound)
        ));
        assert_eq!(runtime.count_active(&owner).await.unwrap(), 0);

        // Stopping again is a harmless no-op (idempotent).
        assert!(runtime.stop(&id).await.is_ok());
    }

    #[tokio::test]
    async fn reaper_kills_a_container_past_its_deadline_but_spares_a_fresh_one() {
        let Some(docker) = docker_or_skip().await else {
            return;
        };
        let runtime = DockerRuntime::with_docker(docker, "runc", true);
        let owner = UserId(format!("reap-owner-{}", std::process::id()));
        let spec = AllocationSpec {
            owner: owner.clone(),
            image: "alpine:latest".to_string(),
            // Free tier: 30-minute lifetime, so the deadline label lands well
            // in the future at creation time.
            limits: PlanLimits::for_tier(PlanTier::Free),
        };

        let id = runtime.create(&spec).await.expect("create a container");
        assert_eq!(runtime.count_active(&owner).await.unwrap(), 1);

        // Reaping "now" spares it -- its deadline is ~30 minutes out.
        let reaped_now = runtime.reap_expired(now_unix()).await.expect("reap now");
        assert!(
            !reaped_now.contains(&id),
            "a fresh container is not past its deadline"
        );
        assert_eq!(
            runtime.status(&id).await.unwrap(),
            AllocationStatus::Running,
            "spared container is still running"
        );

        // Reaping far in the future kills it (deadline has passed) and it's
        // gone -- the hard enforcement of max_lifetime_secs.
        let far_future = now_unix().saturating_add(365 * 24 * 60 * 60);
        let reaped = runtime.reap_expired(far_future).await.expect("reap future");
        assert!(reaped.contains(&id), "an expired container is reaped");
        assert!(matches!(
            runtime.status(&id).await,
            Err(RuntimeError::NotFound)
        ));
        assert_eq!(runtime.count_active(&owner).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn usage_reports_a_real_running_container_with_its_owner() {
        let Some(docker) = docker_or_skip().await else {
            return;
        };
        let runtime = DockerRuntime::with_docker(docker, "runc", true);
        let owner = UserId(format!("usage-owner-{}", std::process::id()));
        let spec = AllocationSpec {
            owner: owner.clone(),
            image: "alpine:latest".to_string(),
            limits: PlanLimits::for_tier(PlanTier::Free),
        };
        let id = runtime.create(&spec).await.expect("create a container");

        let all = runtime.usage().await.expect("usage snapshot");
        let mine = all
            .iter()
            .find(|u| u.id == id)
            .expect("our container appears in usage");
        assert_eq!(mine.owner, owner, "usage is attributed to the real owner");
        // Memory limit should reflect the Free-tier cap (1 GiB) the container
        // was actually created with.
        assert_eq!(mine.memory_limit_bytes, Some(1024 * 1024 * 1024));

        runtime.stop(&id).await.expect("cleanup the container");
    }

    #[tokio::test]
    async fn exec_once_runs_a_command_and_is_owner_scoped() {
        let Some(docker) = docker_or_skip().await else {
            return;
        };
        let runtime = DockerRuntime::with_docker(docker, "runc", true);
        let owner = UserId(format!("exec-owner-{}", std::process::id()));
        let other = UserId(format!("other-owner-{}", std::process::id()));
        let spec = AllocationSpec {
            owner: owner.clone(),
            image: "alpine:latest".to_string(),
            limits: PlanLimits::for_tier(PlanTier::Free),
        };
        let id = runtime.create(&spec).await.expect("create a container");

        // A real command runs and its output + exit code come back.
        let res = runtime
            .exec_once(
                &owner,
                &id,
                &["echo".to_string(), "hello-from-exec".to_string()],
            )
            .await
            .expect("exec runs");
        assert!(
            res.output.contains("hello-from-exec"),
            "real command output: {:?}",
            res.output
        );
        assert_eq!(res.exit_code, Some(0));

        // A non-zero exit is reported honestly.
        let res = runtime
            .exec_once(&owner, &id, &["false".to_string()])
            .await
            .expect("exec runs");
        assert_eq!(res.exit_code, Some(1));

        // OWNER-SCOPING: a different tenant cannot exec into this container.
        let denied = runtime
            .exec_once(&other, &id, &["echo".to_string(), "nope".to_string()])
            .await;
        assert!(
            matches!(denied, Err(RuntimeError::NotFound)),
            "a foreign owner must be denied (NotFound), got {denied:?}"
        );

        // A wholly unknown allocation is NotFound too.
        let unknown = runtime
            .exec_once(
                &owner,
                &AllocationId("nope".to_string()),
                &["true".to_string()],
            )
            .await;
        assert!(matches!(unknown, Err(RuntimeError::NotFound)));

        runtime.stop(&id).await.expect("cleanup the container");
    }

    #[tokio::test]
    async fn spawn_interactive_exec_runs_a_real_shell_and_is_owner_scoped() {
        let Some(docker) = docker_or_skip().await else {
            return;
        };
        let runtime = DockerRuntime::with_docker(docker, "runc", true);
        let owner = UserId(format!("interactive-owner-{}", std::process::id()));
        let other = UserId(format!("interactive-other-{}", std::process::id()));
        let spec = AllocationSpec {
            owner: owner.clone(),
            image: "alpine:latest".to_string(),
            limits: PlanLimits::for_tier(PlanTier::Free),
        };
        let id = runtime.create(&spec).await.expect("create a container");

        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<()>();
        let on_output: Box<dyn FnMut(Vec<u8>) + Send> = Box::new(move |bytes| {
            let _ = out_tx.send(bytes);
        });
        let on_exit: Box<dyn FnOnce() + Send> = Box::new(move || {
            let _ = exit_tx.send(());
        });

        let handle = runtime
            .spawn_interactive_exec(&owner, &id, 80, 24, on_output, on_exit)
            .await
            .expect("a real interactive session starts");

        handle
            .write(b"echo hello-interactive\n".to_vec())
            .expect("write real stdin");

        // Real echoed output must arrive (poll with a bounded timeout so a
        // real failure fails the test instead of hanging forever).
        let mut collected = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), out_rx.recv()).await {
                Ok(Some(chunk)) => {
                    collected.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&collected).contains("hello-interactive") {
                        break;
                    }
                }
                _ => continue,
            }
        }
        assert!(
            String::from_utf8_lossy(&collected).contains("hello-interactive"),
            "real echoed shell output over the interactive session: {:?}",
            String::from_utf8_lossy(&collected)
        );

        // Real resize doesn't error (best-effort, but the call must succeed
        // against a real live session).
        handle.resize(120, 40).expect("resize a live session");

        // Closing stdin (exiting the shell) makes the real session end.
        handle.write(b"exit\n".to_vec()).expect("write exit");
        tokio::time::timeout(std::time::Duration::from_secs(10), exit_rx)
            .await
            .expect("on_exit fires when the real shell exits")
            .expect("exit channel wasn't dropped without firing");

        // OWNER-SCOPING: a different tenant cannot open an interactive
        // session in this container either.
        let (o_tx, _o_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let denied = runtime
            .spawn_interactive_exec(
                &other,
                &id,
                80,
                24,
                Box::new(move |b| {
                    let _ = o_tx.send(b);
                }),
                Box::new(|| {}),
            )
            .await;
        assert!(
            matches!(denied, Err(RuntimeError::NotFound)),
            "a foreign owner must be denied (NotFound), got {denied:?}"
        );

        runtime.stop(&id).await.expect("cleanup the container");
    }
}
