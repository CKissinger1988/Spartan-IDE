import React, { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface DevContainerTerminalProps {
  containerId: string;
}

/**
 * Real §75.74 interactive terminal into a running dev container -- a
 * real sibling of `TerminalView.tsx` (not a generalized version of it):
 * both wrap the same real `xterm.js` instance over a streamed byte
 * pipe, but a container exec session spawns with a `container_id`, not
 * a `cwd`/`command`/`args` triple, and the resulting real Docker `exec`
 * process (not a local child process) has no local PTY of its own to
 * kill -- different enough real spawn/lifecycle shape that a small,
 * separate, honestly-duplicated component was the more maintainable
 * choice than bending `TerminalView`'s own already-working, tested
 * local-PTY logic to cover a second, structurally different case.
 */
export default function DevContainerTerminal({
  containerId,
}: DevContainerTerminalProps): React.ReactElement {
  const containerRef = useRef<HTMLDivElement>(null);
  const sessionIdRef = useRef<number | null>(null);

  useEffect(() => {
    const term = new Terminal({
      fontFamily: "SF Mono, JetBrains Mono, Cascadia Code, Consolas, monospace",
      fontSize: 13,
      theme: {
        background: "#09090b",
        foreground: "#e9e7e4",
        cursor: "#2e7dff",
      },
      convertEol: true,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    if (containerRef.current) {
      term.open(containerRef.current);
      fitAddon.fit();
    }

    let disposed = false;
    let unsubscribe: (() => void) | null = null;

    window.spartan
      .call("devcontainer_exec_spawn", {
        container_id: containerId,
        cols: term.cols,
        rows: term.rows,
      })
      .then((result) => {
        if (disposed) return;
        const sessionId = (result as { session_id: number }).session_id;
        sessionIdRef.current = sessionId;

        unsubscribe = window.spartan.onEvent((event, data) => {
          const d = data as { session_id?: number; chunk?: string };
          if (d.session_id !== sessionId) return;
          if (event === "devcontainer_exec_output" && d.chunk) {
            term.write(d.chunk);
          } else if (event === "devcontainer_exec_exit") {
            term.write("\r\n[container session ended]\r\n");
          }
        });

        term.onData((data) => {
          window.spartan.call("devcontainer_exec_input", { session_id: sessionId, data }).catch(() => {});
        });
      })
      .catch((e: Error) => {
        term.write(`Failed to start container session: ${e.message}\r\n`);
      });

    const handleResize = () => {
      fitAddon.fit();
      if (sessionIdRef.current !== null) {
        window.spartan
          .call("devcontainer_exec_resize", {
            session_id: sessionIdRef.current,
            cols: term.cols,
            rows: term.rows,
          })
          .catch(() => {});
      }
    };
    window.addEventListener("resize", handleResize);
    const resizeObserver = new ResizeObserver(handleResize);
    if (containerRef.current) resizeObserver.observe(containerRef.current);

    return () => {
      disposed = true;
      window.removeEventListener("resize", handleResize);
      resizeObserver.disconnect();
      unsubscribe?.();
      if (sessionIdRef.current !== null) {
        window.spartan.call("devcontainer_exec_close", { session_id: sessionIdRef.current }).catch(() => {});
      }
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [containerId]);

  return <div className="terminal-view" ref={containerRef} />;
}
