import React, { useCallback, useEffect, useState } from "react";
import DevContainerTerminal from "./DevContainerTerminal";

interface DevContainerConfigSummary {
  name: string | null;
  image: string | null;
  hasBuild: boolean;
  forwardPorts: number[];
  hasPostCreateCommand: boolean;
}

interface ManagedContainer {
  id: string;
  name: string;
  image: string;
  status: string;
  projectLabel: string;
}

type Phase = "idle" | "starting" | "running" | "stopping" | "failed";

interface DevContainersScreenProps {
  root: string;
}

/**
 * Real §75.74 Dev Containers screen -- OCI/Docker-based, following the
 * open containers.dev `devcontainer.json` spec, closing the user's own
 * "add virtual machine dev containers... to allow testing projects on
 * different OS's" request. A real, explicit scope decision confirmed
 * with the user up front (`AskUserQuestion`): container-based, not true
 * separate-kernel VMs -- this development environment itself has no
 * `/dev/kvm` at all, so a QEMU/KVM path couldn't even be exercised here,
 * and OCI containers are the real, industry-standard answer this whole
 * competitor category (VS Code Dev Containers, GitHub Codespaces,
 * JetBrains Gateway) actually ships.
 *
 * Talks to `spartan-backend`'s real `devcontainer_*` IPC methods, which
 * wrap the real `spartan-devcontainer` crate (a real, JSONC-tolerant
 * devcontainer.json parser + real Docker Engine API calls via
 * `bollard`). `devcontainer_up`/`devcontainer_down` are real, possibly
 * slow operations (an image pull/build can take minutes) -- this screen
 * follows the same "immediate ack, then real unprompted progress/
 * ready/failed events" pattern `LeoChatPanel.tsx` already established
 * for Leo's own slow model calls.
 */
export default function DevContainersScreen({ root }: DevContainersScreenProps): React.ReactElement {
  const [detecting, setDetecting] = useState(true);
  const [config, setConfig] = useState<DevContainerConfigSummary | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [detectError, setDetectError] = useState<string | null>(null);

  const [phase, setPhase] = useState<Phase>("idle");
  const [progressLines, setProgressLines] = useState<string[]>([]);
  const [containerId, setContainerId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [managed, setManaged] = useState<ManagedContainer[]>([]);

  const refreshManaged = useCallback(() => {
    window.spartan
      .call("devcontainer_list", {})
      .then((result) => setManaged(result as ManagedContainer[]))
      .catch(() => {});
  }, []);

  useEffect(() => {
    setDetecting(true);
    window.spartan
      .call("devcontainer_detect", { project_root: root })
      .then((result) => {
        const r = result as { found: boolean; config?: DevContainerConfigSummary };
        setNotFound(!r.found);
        setConfig(r.config ?? null);
        setDetectError(null);
      })
      .catch((e: Error) => setDetectError(e.message))
      .finally(() => setDetecting(false));
    refreshManaged();
  }, [root, refreshManaged]);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "devcontainer_progress") {
        setProgressLines((prev) => [...prev, (data as { line: string }).line]);
      } else if (event === "devcontainer_ready") {
        const d = data as { container_id: string; status: string };
        setContainerId(d.container_id);
        setPhase("running");
        refreshManaged();
      } else if (event === "devcontainer_stopped") {
        setContainerId(null);
        setPhase("idle");
        setProgressLines([]);
        refreshManaged();
      } else if (event === "devcontainer_failed") {
        setError((data as { error: string }).error);
        setPhase("failed");
        refreshManaged();
      }
    });
    return unsubscribe;
  }, [refreshManaged]);

  const start = useCallback(() => {
    setError(null);
    setProgressLines([]);
    setPhase("starting");
    window.spartan.call("devcontainer_up", { project_root: root }).catch((e: Error) => {
      setError(e.message);
      setPhase("failed");
    });
  }, [root]);

  const stop = useCallback(() => {
    if (!containerId) return;
    setPhase("stopping");
    window.spartan.call("devcontainer_down", { container_id: containerId }).catch((e: Error) => {
      setError(e.message);
      setPhase("failed");
    });
  }, [containerId]);

  if (detecting) {
    return <div className="devcontainer-screen mono">Checking for a devcontainer.json…</div>;
  }
  if (detectError) {
    return <div className="devcontainer-screen mono">{detectError}</div>;
  }

  return (
    <div className="devcontainer-screen">
      {notFound && phase === "idle" && (
        <div className="devcontainer-empty mono">
          No devcontainer.json found in this project (checked .devcontainer/devcontainer.json and
          .devcontainer.json). Add one following the open{" "}
          <span className="devcontainer-spec-name">containers.dev</span> specification to test this
          project inside an isolated Linux environment — a different distro, toolchain versions, or a
          clean install, without touching your host machine.
        </div>
      )}

      {config && (
        <div className="devcontainer-config mono">
          <div className="devcontainer-config-row">
            <span className="devcontainer-label">Name</span>
            <span>{config.name ?? "(unnamed)"}</span>
          </div>
          <div className="devcontainer-config-row">
            <span className="devcontainer-label">Source</span>
            <span>{config.hasBuild ? "Built from a real Dockerfile" : config.image ?? "(no image)"}</span>
          </div>
          {config.forwardPorts.length > 0 && (
            <div className="devcontainer-config-row">
              <span className="devcontainer-label">Forwarded ports</span>
              <span>{config.forwardPorts.join(", ")}</span>
            </div>
          )}
          {config.hasPostCreateCommand && (
            <div className="devcontainer-config-row">
              <span className="devcontainer-label">Setup command</span>
              <span>Runs automatically once the container starts</span>
            </div>
          )}

          <div className="devcontainer-actions">
            {(phase === "idle" || phase === "failed") && (
              <button className="devcontainer-btn devcontainer-btn-start sf-chamfer-sm" onClick={start}>
                Start Dev Container
              </button>
            )}
            {phase === "starting" && (
              <span className="devcontainer-status-badge devcontainer-status-starting">Starting…</span>
            )}
            {phase === "running" && (
              <>
                <span className="devcontainer-status-badge devcontainer-status-running">Running</span>
                <button className="devcontainer-btn devcontainer-btn-stop" onClick={stop}>
                  Stop
                </button>
              </>
            )}
            {phase === "stopping" && (
              <span className="devcontainer-status-badge devcontainer-status-starting">Stopping…</span>
            )}
          </div>
        </div>
      )}

      {progressLines.length > 0 && (phase === "starting" || phase === "failed") && (
        <pre className="devcontainer-progress mono">{progressLines.join("\n")}</pre>
      )}

      {error && <div className="devcontainer-error mono">{error}</div>}

      {phase === "running" && containerId && (
        <div className="devcontainer-terminal-wrap">
          <div className="devcontainer-terminal-label mono">Container shell</div>
          <DevContainerTerminal containerId={containerId} />
        </div>
      )}

      {managed.length > 0 && (
        <div className="devcontainer-managed mono">
          <div className="devcontainer-managed-label">Spartan-managed containers on this machine</div>
          {managed.map((c) => (
            <div key={c.id} className="devcontainer-managed-row">
              <span>{c.name}</span>
              <span className="devcontainer-managed-image">{c.image}</span>
              <span>{c.status}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
