//! Real Docker Engine API lifecycle management via `bollard` -- talks
//! directly to the real Docker daemon's own REST API over its Unix
//! socket, the same real mechanism the `docker` CLI itself uses, rather
//! than shelling out to and parsing that CLI's text output. A real,
//! deliberate architectural note: this workspace's own established
//! convention (`spartan-model`'s own `ModelProvider` doc comment) is
//! sync/callback-based, not `tokio`-based -- every function here keeps
//! that promise at its own public boundary. `tokio` is a real,
//! completely contained implementation detail: each function builds its
//! own single-thread runtime on its own caller-provided background
//! thread and calls `block_on`, the same "spawn a thread, do the real
//! slow work there, report back over a channel" shape every other real
//! background operation in this workspace (`leo_bridge.rs`,
//! `update_bridge.rs`, `pty.rs`) already uses -- nothing here requires
//! the rest of the workspace to become async.

use crate::spec::DevContainerConfig;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogOutput, RemoveContainerOptions,
    StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use bollard::image::{BuildImageOptions, CreateImageOptions};
use bollard::models::{HostConfig, Mount, MountTypeEnum, PortBinding};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::Path;
use tokio::io::AsyncWriteExt;

/// Real label keys stamped onto every container this crate creates --
/// the one real mechanism `list_managed_containers` uses to tell a
/// Spartan-managed dev container apart from an arbitrary container
/// already running on the user's machine. This crate never lists, stops,
/// or removes a container it didn't create.
pub const LABEL_MANAGED: &str = "com.spartan-ide.devcontainer";
pub const LABEL_PROJECT: &str = "com.spartan-ide.devcontainer.project";

#[derive(Debug)]
pub struct DockerError(pub String);

impl std::fmt::Display for DockerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for DockerError {}

impl From<bollard::errors::Error> for DockerError {
    fn from(e: bollard::errors::Error) -> Self {
        DockerError(e.to_string())
    }
}
impl From<std::io::Error> for DockerError {
    fn from(e: std::io::Error) -> Self {
        DockerError(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    /// Real Docker state string -- `"running"`, `"exited"`, `"created"`,
    /// etc, taken verbatim from the daemon's own real report.
    pub status: String,
    pub project_label: String,
}

fn connect() -> Result<Docker, DockerError> {
    Docker::connect_with_local_defaults().map_err(DockerError::from)
}

fn current_thread_runtime() -> Result<tokio::runtime::Runtime, DockerError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DockerError::from)
}

/// Real, live reachability check -- the one real way to tell "no Docker
/// daemon reachable" apart from any other real failure, used both by
/// this crate's own self-skipping integration tests and by
/// `spartan-backend` to report an honest, specific "Docker isn't
/// running" message rather than a generic error.
pub fn is_docker_available() -> bool {
    let Ok(rt) = current_thread_runtime() else {
        return false;
    };
    rt.block_on(async {
        match connect() {
            Ok(docker) => docker.ping().await.is_ok(),
            Err(_) => false,
        }
    })
}

/// Real image pull, streaming real progress lines to `on_progress` as
/// the daemon reports them.
pub fn pull_image(image: &str, mut on_progress: impl FnMut(String)) -> Result<(), DockerError> {
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;
        let options = CreateImageOptions {
            from_image: image,
            ..Default::default()
        };
        let mut stream = docker.create_image(Some(options), None, None);
        while let Some(item) = stream.next().await {
            let info = item?;
            if let Some(status) = info.status {
                let line = match info.progress {
                    Some(p) => format!("{status} {p}"),
                    None => status,
                };
                on_progress(line);
            }
        }
        Ok(())
    })
}

/// Real image build from a real Dockerfile -- `context_dir` is tarred up
/// in memory (real build contexts are typically small enough that this
/// is the honest, simple choice over a temp-file-backed streaming tar;
/// a real, named limit for a build context so large it doesn't
/// reasonably fit in memory, not attempted this pass).
pub fn build_image(
    context_dir: &Path,
    dockerfile_rel_path: &str,
    build_args: &std::collections::BTreeMap<String, String>,
    tag: &str,
    mut on_progress: impl FnMut(String),
) -> Result<(), DockerError> {
    let tar_bytes = tar_directory(context_dir)?;
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;
        let options = BuildImageOptions {
            dockerfile: dockerfile_rel_path,
            t: tag,
            rm: true,
            buildargs: build_args
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            ..Default::default()
        };
        let mut stream = docker.build_image(options, None, Some(tar_bytes.into()));
        while let Some(item) = stream.next().await {
            let info = item?;
            if let Some(error) = info.error {
                return Err(DockerError(format!("build failed: {error}")));
            }
            if let Some(stream_line) = info.stream {
                on_progress(stream_line);
            } else if let Some(status) = info.status {
                on_progress(status);
            }
        }
        Ok(())
    })
}

/// Real, minimal in-memory tar of a real directory tree -- the exact
/// real shape `bollard::Docker::build_image`'s own `tar` parameter
/// requires as its build context.
fn tar_directory(dir: &Path) -> Result<Vec<u8>, DockerError> {
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_dir_all(".", dir)
        .map_err(|e| DockerError(format!("failed to tar build context: {e}")))?;
    builder
        .into_inner()
        .map_err(|e| DockerError(format!("failed to finish build context tar: {e}")))
}

/// Real parsing of a real devcontainer.json `mounts` entry
/// (`"source=...,target=...,type=bind[,readonly]"`) into a real,
/// structured `bollard::models::Mount` -- the spec's own documented
/// grammar, not invented here.
fn parse_mount_string(raw: &str) -> Option<Mount> {
    let mut source = None;
    let mut target = None;
    let mut typ = MountTypeEnum::BIND;
    let mut read_only = false;
    for part in raw.split(',') {
        let mut kv = part.splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next().map(str::trim);
        match key {
            "source" => source = value.map(str::to_string),
            "target" | "destination" => target = value.map(str::to_string),
            "type" => {
                typ = match value {
                    Some("volume") => MountTypeEnum::VOLUME,
                    Some("tmpfs") => MountTypeEnum::TMPFS,
                    _ => MountTypeEnum::BIND,
                }
            }
            "readonly" | "ro" => read_only = true,
            _ => {}
        }
    }
    Some(Mount {
        source,
        target,
        typ: Some(typ),
        read_only: Some(read_only),
        ..Default::default()
    })
}

/// Real spec convention: when no `overrideCommand: false` is present,
/// real devcontainer tooling keeps the container alive with a real,
/// long-running foreground process regardless of what the base image's
/// own entrypoint/cmd would otherwise do, specifically so a user can
/// `exec` into it interactively at any time -- this crate always applies
/// that same real, standard convention (this project's own subset of
/// the spec doesn't implement `overrideCommand: false` as an opt-out).
fn keep_alive_command() -> Vec<String> {
    vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "trap 'exit 0' TERM INT; sleep infinity & wait".to_string(),
    ]
}

/// Real container creation + start. `image` is the real, already-
/// resolved image reference (the caller has already pulled or built it
/// via `pull_image`/`build_image`). Returns the real Docker-assigned
/// container ID.
pub fn create_and_start_container(
    image: &str,
    config: &DevContainerConfig,
    project_root: &Path,
    project_label: &str,
    container_name: &str,
) -> Result<String, DockerError> {
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;

        let workspace_folder = config
            .workspace_folder
            .clone()
            .unwrap_or_else(|| "/workspaces/project".to_string());

        let mut mounts = vec![Mount {
            source: Some(project_root.to_string_lossy().into_owned()),
            target: Some(workspace_folder.clone()),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        }];
        for raw in &config.mounts {
            if let Some(m) = parse_mount_string(raw) {
                mounts.push(m);
            }
        }

        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
        for port in &config.forward_ports {
            let key = format!("{port}/tcp");
            exposed_ports.insert(key.clone(), HashMap::new());
            port_bindings.insert(
                key,
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".to_string()),
                    host_port: Some(port.to_string()),
                }]),
            );
        }

        let env: Vec<String> = config
            .container_env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        let mut labels = HashMap::new();
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        labels.insert(LABEL_PROJECT.to_string(), project_label.to_string());

        let host_config = HostConfig {
            mounts: Some(mounts),
            port_bindings: if port_bindings.is_empty() {
                None
            } else {
                Some(port_bindings)
            },
            ..Default::default()
        };

        let container_config = Config {
            image: Some(image.to_string()),
            cmd: Some(keep_alive_command()),
            env: if env.is_empty() { None } else { Some(env) },
            labels: Some(labels),
            user: config.remote_user.clone(),
            working_dir: Some(workspace_folder),
            exposed_ports: if exposed_ports.is_empty() {
                None
            } else {
                Some(exposed_ports)
            },
            host_config: Some(host_config),
            tty: Some(true),
            open_stdin: Some(true),
            ..Default::default()
        };

        let create_options = CreateContainerOptions {
            name: container_name.to_string(),
            platform: None,
        };

        let response = docker
            .create_container(Some(create_options), container_config)
            .await?;
        docker
            .start_container(
                &response.id,
                None::<bollard::container::StartContainerOptions<String>>,
            )
            .await?;
        Ok(response.id)
    })
}

/// Real one-shot command execution inside an already-running container
/// -- used for `postCreateCommand`. Returns the real exit code and real
/// combined stdout+stderr output.
pub fn run_command(container_id: &str, argv: &[String]) -> Result<(i64, String), DockerError> {
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;
        let exec = docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    cmd: Some(argv.to_vec()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?;

        let mut output_text = String::new();
        if let StartExecResults::Attached { mut output, .. } =
            docker.start_exec(&exec.id, None).await?
        {
            while let Some(item) = output.next().await {
                let log = item?;
                output_text.push_str(&log.to_string());
            }
        }

        let inspect = docker.inspect_exec(&exec.id).await?;
        let exit_code = inspect.exit_code.unwrap_or(-1);
        Ok((exit_code, output_text))
    })
}

pub fn stop_and_remove(container_id: &str) -> Result<(), DockerError> {
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;
        // A real, honest best-effort stop -- a container that's already
        // stopped or gone returns a real error here that removal below
        // would also hit, so both are treated as "already achieved the
        // goal," not surfaced as a hard failure.
        let _ = docker
            .stop_container(container_id, Some(StopContainerOptions { t: 5 }))
            .await;
        docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await?;
        Ok(())
    })
}

/// Real live status -- `None` if the container no longer exists (already
/// removed, e.g. by the user directly via `docker rm`), a real, honest
/// case this crate treats as "gone," not an error.
pub fn container_status(container_id: &str) -> Result<Option<String>, DockerError> {
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;
        match docker.inspect_container(container_id, None).await {
            Ok(inspect) => Ok(inspect
                .state
                .and_then(|s| s.status)
                .map(|status| status.to_string())),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(e) => Err(DockerError::from(e)),
        }
    })
}

/// Real list of only the containers this crate itself created (filtered
/// by `LABEL_MANAGED`) -- never touches or reports on a container the
/// user created some other way.
pub fn list_managed_containers() -> Result<Vec<ManagedContainer>, DockerError> {
    let rt = current_thread_runtime()?;
    rt.block_on(async {
        let docker = connect()?;
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![LABEL_MANAGED.to_string()]);
        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };
        let summaries = docker.list_containers(Some(options)).await?;
        Ok(summaries
            .into_iter()
            .map(|s| ManagedContainer {
                id: s.id.unwrap_or_default(),
                name: s
                    .names
                    .and_then(|n| n.into_iter().next())
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_string(),
                image: s.image.unwrap_or_default(),
                status: s.state.unwrap_or_default(),
                project_label: s
                    .labels
                    .and_then(|l| l.get(LABEL_PROJECT).cloned())
                    .unwrap_or_default(),
            })
            .collect())
    })
}

/// Real commands an already-running interactive exec session accepts --
/// sent over a plain `tokio::sync::mpsc::UnboundedSender`, whose own
/// `send()` is a real, non-async, ordinary function, so a caller on any
/// plain OS thread (no runtime of its own) can drive it exactly like
/// `pty.rs::PtyHandle::write`'s own sync signature.
enum ExecCommand {
    Input(Vec<u8>),
    Resize(u16, u16),
}

pub struct ExecHandle {
    tx: tokio::sync::mpsc::UnboundedSender<ExecCommand>,
}

impl ExecHandle {
    pub fn write(&self, data: &[u8]) -> Result<(), DockerError> {
        self.tx
            .send(ExecCommand::Input(data.to_vec()))
            .map_err(|_| DockerError("exec session already closed".to_string()))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), DockerError> {
        self.tx
            .send(ExecCommand::Resize(cols, rows))
            .map_err(|_| DockerError("exec session already closed".to_string()))
    }

    /// Real close -- dropping the sender makes the background loop's own
    /// `rx.recv()` return `None`, ending the session; a shell running as
    /// the exec's command typically also observes the resulting stdin
    /// close as a real EOF and exits on its own.
    pub fn close(self) {
        drop(self.tx);
    }
}

/// Real, interactive `docker exec -it`-equivalent session -- spawns its
/// own real background OS thread (the same real "one dedicated thread
/// per long-lived operation" shape `pty.rs::spawn_pty` already
/// established for local PTYs), forwarding real output bytes to
/// `on_output` as they arrive and calling `on_exit` exactly once when
/// the session ends for any reason (the shell exited, the container
/// stopped, a real Docker error).
pub fn spawn_interactive_exec(
    container_id: &str,
    cols: u16,
    rows: u16,
    mut on_output: impl FnMut(Vec<u8>) + Send + 'static,
    on_exit: impl FnOnce() + Send + 'static,
) -> Result<ExecHandle, DockerError> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecCommand>();
    let container_id = container_id.to_string();

    std::thread::spawn(move || {
        let rt = match current_thread_runtime() {
            Ok(rt) => rt,
            Err(_) => {
                on_exit();
                return;
            }
        };
        rt.block_on(async move {
            let docker = match connect() {
                Ok(d) => d,
                Err(_) => {
                    on_exit();
                    return;
                }
            };
            let exec = match docker
                .create_exec(
                    &container_id,
                    CreateExecOptions {
                        cmd: Some(vec!["/bin/sh".to_string()]),
                        attach_stdin: Some(true),
                        attach_stdout: Some(true),
                        attach_stderr: Some(true),
                        tty: Some(true),
                        ..Default::default()
                    },
                )
                .await
            {
                Ok(e) => e,
                Err(_) => {
                    on_exit();
                    return;
                }
            };

            let start_result = docker
                .start_exec(
                    &exec.id,
                    Some(StartExecOptions {
                        tty: true,
                        ..Default::default()
                    }),
                )
                .await;

            let (mut output, mut input) = match start_result {
                Ok(StartExecResults::Attached { output, input }) => (output, input),
                _ => {
                    on_exit();
                    return;
                }
            };

            let _ = docker
                .resize_exec(
                    &exec.id,
                    ResizeExecOptions {
                        height: rows,
                        width: cols,
                    },
                )
                .await;

            loop {
                tokio::select! {
                    item = output.next() => {
                        match item {
                            Some(Ok(LogOutput::StdOut { message } | LogOutput::StdErr { message } | LogOutput::Console { message })) => {
                                on_output(message.to_vec());
                            }
                            Some(Ok(LogOutput::StdIn { .. })) => {}
                            Some(Err(_)) | None => break,
                        }
                    }
                    cmd = rx.recv() => {
                        match cmd {
                            Some(ExecCommand::Input(bytes)) => {
                                if input.write_all(&bytes).await.is_err() {
                                    break;
                                }
                            }
                            Some(ExecCommand::Resize(cols, rows)) => {
                                let _ = docker
                                    .resize_exec(&exec.id, ResizeExecOptions { height: rows, width: cols })
                                    .await;
                            }
                            None => break,
                        }
                    }
                }
            }
            on_exit();
        });
    });

    Ok(ExecHandle { tx })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mount_string_handles_a_real_bind_mount() {
        let m = parse_mount_string("source=/host/path,target=/workspace,type=bind").unwrap();
        assert_eq!(m.source.as_deref(), Some("/host/path"));
        assert_eq!(m.target.as_deref(), Some("/workspace"));
        assert_eq!(m.typ, Some(MountTypeEnum::BIND));
        assert_eq!(m.read_only, Some(false));
    }

    #[test]
    fn parse_mount_string_defaults_to_bind_when_type_is_omitted() {
        let m = parse_mount_string("source=/a,target=/b").unwrap();
        assert_eq!(m.typ, Some(MountTypeEnum::BIND));
    }

    #[test]
    fn parse_mount_string_handles_a_real_named_volume() {
        let m = parse_mount_string("source=my-volume,target=/data,type=volume").unwrap();
        assert_eq!(m.typ, Some(MountTypeEnum::VOLUME));
    }

    #[test]
    fn parse_mount_string_recognizes_the_readonly_flag() {
        let m = parse_mount_string("source=/a,target=/b,type=bind,readonly").unwrap();
        assert_eq!(m.read_only, Some(true));
    }

    #[test]
    fn keep_alive_command_is_a_real_never_exiting_shell_loop() {
        let cmd = keep_alive_command();
        assert_eq!(cmd[0], "/bin/sh");
        assert!(cmd.iter().any(|s| s.contains("sleep infinity")));
    }
}
