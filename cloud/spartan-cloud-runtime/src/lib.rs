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
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions, StatsOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{ContainerStateStatusEnum, HostConfig};
use bollard::Docker;
use futures_util::StreamExt;

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
}
