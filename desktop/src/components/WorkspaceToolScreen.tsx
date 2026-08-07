import React, { useCallback, useEffect, useState } from "react";
import { NAV, type ScreenId } from "../nav";

interface WorkspaceToolScreenProps {
  screen: ScreenId;
  root: string;
  onOpenFile: (path: string) => void;
}

interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
}

interface ResourceCatalogEntry {
  name: string;
  description: string;
  source: string;
  kind: "skill" | "mcp" | "plugin";
  official?: boolean;
}

interface InstalledResource {
  name: string;
  path: string;
  target: "project" | "user";
  valid: boolean;
  detail: string;
}

const COMMON_SKILLS: ResourceCatalogEntry[] = [
  { name: "OpenAI Plugins", description: "Curated Codex plugin examples with skills, MCP, commands, and hooks.", source: "https://github.com/openai/plugins", kind: "skill", official: true },
  { name: "Hugging Face Skills", description: "Agent skills for the Hugging Face ecosystem.", source: "https://github.com/huggingface/skills", kind: "skill" },
  { name: "OpenAI Skills Archive", description: "Legacy curated and experimental Codex skills catalog.", source: "https://github.com/openai/skills", kind: "skill", official: true },
];

const COMMON_MCP: ResourceCatalogEntry[] = [
  { name: "Filesystem", description: "Secure filesystem access with explicit allowed directories.", source: "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem", kind: "mcp", official: true },
  { name: "Git", description: "Read and manipulate Git repositories through MCP.", source: "https://github.com/modelcontextprotocol/servers/tree/main/src/git", kind: "mcp", official: true },
  { name: "Fetch", description: "Fetch and convert web content for agent use.", source: "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch", kind: "mcp", official: true },
  { name: "Memory", description: "Persistent knowledge graph memory server.", source: "https://github.com/modelcontextprotocol/servers/tree/main/src/memory", kind: "mcp", official: true },
  { name: "Time", description: "Time and timezone conversion server.", source: "https://github.com/modelcontextprotocol/servers/tree/main/src/time", kind: "mcp", official: true },
  { name: "GitHub MCP Server", description: "GitHub's official MCP server for repository and issue workflows.", source: "https://github.com/github/github-mcp-server", kind: "mcp" },
];

const COMMON_PLUGINS: ResourceCatalogEntry[] = [
  { name: "Build Web Apps", description: "OpenAI's web application building workflow and guidance.", source: "https://github.com/openai/plugins/tree/main/plugins/build-web-apps", kind: "plugin", official: true },
  { name: "GitHub", description: "Repository, issue, pull request, and project workflows for GitHub.", source: "https://github.com/openai/plugins/tree/main/plugins/github", kind: "plugin", official: true },
  { name: "Figma", description: "Design-file context and collaboration workflows for Figma.", source: "https://github.com/openai/plugins/tree/main/plugins/figma", kind: "plugin", official: true },
  { name: "Notion", description: "Workspace search and knowledge workflows for Notion.", source: "https://github.com/openai/plugins/tree/main/plugins/notion", kind: "plugin", official: true },
  { name: "Cloudflare", description: "Cloudflare development and deployment workflows.", source: "https://github.com/openai/plugins/tree/main/plugins/cloudflare", kind: "plugin", official: true },
  { name: "Game Studio", description: "Browser game development workflows and assets.", source: "https://github.com/openai/plugins/tree/main/plugins/game-studio", kind: "plugin", official: true },
];

function screenLabel(screen: ScreenId): string {
  return NAV.flatMap((group) => group.items).find((item) => item.id === screen)?.label ?? screen;
}

function summarize(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/** Real workspace-backed destination for navigation entries that do not have
 * a dedicated specialist screen yet. It deliberately uses the existing
 * shell's plain screen chrome while exposing live project data and a real
 * refresh action instead of claiming a feature is wired when it is not. */
export default function WorkspaceToolScreen({ screen, root, onOpenFile }: WorkspaceToolScreenProps): React.ReactElement {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [result, setResult] = useState<string>("Loading workspace data…");
  const [loading, setLoading] = useState(false);
  const [catalog, setCatalog] = useState<ResourceCatalogEntry[]>([]);
  const [resourceSource, setResourceSource] = useState("");
  const [resourceName, setResourceName] = useState("");
  const [resourceTarget, setResourceTarget] = useState<"project" | "user">("project");
  const [installing, setInstalling] = useState<string | null>(null);
  const [installed, setInstalled] = useState<InstalledResource[]>([]);

  const loadInstalled = useCallback(async () => {
    if (!["skills", "mcp", "plugins", "marketplace"].includes(screen)) return;
    try {
      const kind: ResourceCatalogEntry["kind"] = screen === "skills" ? "skill" : screen === "mcp" ? "mcp" : "plugin";
      const response = await window.spartan.call("resource_list", { kind }) as { installed?: InstalledResource[] };
      setInstalled(response.installed ?? []);
    } catch (error) {
      setResult(`Installed resources unavailable: ${(error as Error).message}`);
    }
  }, [screen]);

  const loadCatalog = useCallback(async () => {
    if (!["skills", "mcp", "plugins", "marketplace"].includes(screen)) return;
    const kind: ResourceCatalogEntry["kind"] = screen === "skills" ? "skill" : screen === "mcp" ? "mcp" : "plugin";
    const fallback = kind === "skill" ? COMMON_SKILLS : kind === "mcp" ? COMMON_MCP : COMMON_PLUGINS;
    try {
      if (kind === "skill" || kind === "plugin") {
        const response = await fetch("https://api.github.com/repos/openai/plugins/contents/plugins", { headers: { Accept: "application/vnd.github+json" } });
        if (!response.ok) throw new Error(`GitHub returned HTTP ${response.status}`);
        const items = await response.json() as Array<{ name?: string; type?: string }>;
        const live = items.filter((item) => item.type === "dir" && item.name).map((item) => ({
          name: item.name!, description: kind === "plugin" ? "OpenAI plugin with discoverable skills and optional MCP surfaces." : "OpenAI skill or plugin resource.", source: `https://github.com/openai/plugins/tree/main/plugins/${item.name}`, kind, official: true,
        }));
        setCatalog([...fallback, ...live.filter((item) => !fallback.some((entry) => entry.name.toLowerCase() === item.name.toLowerCase()))]);
      } else {
        const response = await fetch("https://registry.modelcontextprotocol.io/v0.1/servers?version=latest&limit=40");
        if (!response.ok) throw new Error(`MCP Registry returned HTTP ${response.status}`);
        const payload = await response.json() as { servers?: Array<{ server?: { name?: string; description?: string; repository?: { url?: string } } }> };
        const live = (payload.servers ?? []).flatMap((item) => {
          const server = item.server;
          return server?.name && server.repository?.url ? [{ name: server.name, description: server.description ?? "Registry-published MCP server.", source: server.repository.url, kind: "mcp" as const }] : [];
        });
        setCatalog([...fallback, ...live.filter((item) => !fallback.some((entry) => entry.name === item.name))]);
      }
      await loadInstalled();
    } catch (error) {
      setCatalog(fallback);
      setResult(`Live catalog unavailable; showing the built-in common catalog. ${(error as Error).message}`);
    }
  }, [loadInstalled, screen]);

  const installResource = useCallback(async (entry: ResourceCatalogEntry, source = entry.source, name = entry.name) => {
    setInstalling(name);
    try {
      await window.spartan.call("resource_install", { kind: entry.kind, name, source, target: resourceTarget });
      setResult(`Installed ${entry.kind} "${name}" to the ${resourceTarget} catalog.`);
      setResourceSource("");
      setResourceName("");
      await loadInstalled();
    } catch (error) {
      setResult(`Install failed: ${(error as Error).message}`);
    } finally {
      setInstalling(null);
    }
  }, [loadInstalled, resourceTarget]);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const directory = (await window.spartan.call("list_dir", { path: root })) as { entries?: Entry[] };
      const nextEntries = directory.entries ?? [];
      setEntries(nextEntries);

      if (["skills", "mcp", "plugins", "marketplace"].includes(screen)) {
        await loadCatalog();
        return;
      }

      if (screen === "review") {
        const status = await window.spartan.call("git_status", { project_root: root });
        setResult(summarize(status));
      } else if (screen === "analytics" || screen === "usage") {
        const log = await window.spartan.call("git_log", { project_root: root, max: 50 });
        setResult(summarize(log));
      } else if (screen === "agents") {
        const sessions = await window.spartan.call("leo_list_sessions", {});
        setResult(summarize(sessions));
      } else {
        const relevant = nextEntries.filter((entry) => {
          const name = entry.name.toLowerCase();
          if (screen === "commands") return name.includes("command") || name === ".spartan";
          if (screen === "hooks") return name.includes("hook") || name === ".git hooks";
          if (screen === "routing") return name.includes("route") || name.includes("config");
          if (screen === "plugins") return name.includes("plugin");
          if (screen === "marketplace") return name.includes("market") || name.includes("catalog");
          return true;
        });
        setResult(relevant.length ? relevant.map((entry) => `${entry.is_dir ? "DIR " : "FILE"} ${entry.path}`).join("\n") : "No matching project resources found.");
      }
    } catch (error) {
      setResult(`Unable to load ${screenLabel(screen)} data: ${(error as Error).message}`);
    } finally {
      setLoading(false);
    }
  }, [loadCatalog, root, screen]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const fileEntries = entries.filter((entry) => !entry.is_dir);

  if (["skills", "mcp", "plugins", "marketplace"].includes(screen)) {
    const kind: ResourceCatalogEntry["kind"] = screen === "skills" ? "skill" : screen === "mcp" ? "mcp" : "plugin";
    const resourceLabel = screen === "skills" ? "Skills" : screen === "mcp" ? "MCP Resources" : screen === "marketplace" ? "Marketplace" : "Plugins";
    return (
      <section className="workspace-tool-screen resource-screen" aria-label={`${screenLabel(screen)} catalog`}>
        <h2 className="mono">{screenLabel(screen)}</h2>
        <p className="workspace-tool-summary">Live {resourceLabel.toLowerCase()} catalog plus project and user installation targets.</p>
        <p className="resource-help">New to this? Install a catalog item to add it to this project, or paste a GitHub/HTTPS/local path for a custom resource. Project installs stay with the workspace; user installs are reusable across projects.</p>
        <div className="resource-install-bar">
          <input className="resource-input mono" aria-label={`${kind} source`} placeholder="GitHub, HTTPS file, or local path" value={resourceSource} onChange={(event) => setResourceSource(event.target.value)} />
          <input className="resource-input mono" aria-label={`${kind} name`} placeholder="Install name" value={resourceName} onChange={(event) => setResourceName(event.target.value)} />
          <select className="resource-target mono" value={resourceTarget} onChange={(event) => setResourceTarget(event.target.value as "project" | "user")}>
            <option value="project">Project .spartan</option>
            <option value="user">User catalog</option>
          </select>
          <button className="toolbar-btn toolbar-btn-primary" type="button" disabled={!resourceSource.trim() || !resourceName.trim() || installing !== null} onClick={() => void installResource({ name: resourceName, description: "Custom resource", source: resourceSource, kind }, resourceSource, resourceName)}>
            Install custom
          </button>
        </div>
        <div className="resource-catalog mono">
          {catalog.map((entry) => (
            <article className="resource-card" key={`${entry.kind}:${entry.name}`}>
              <div className="resource-card-heading"><strong>{entry.name}</strong>{entry.official && <span className="resource-badge">official</span>}</div>
              <p>{entry.description}</p>
              <div className="resource-card-source" title={entry.source}>{entry.source}</div>
              <button className="toolbar-btn" type="button" disabled={installing !== null} onClick={() => void installResource(entry)}>{installing === entry.name ? "Installing…" : `Install ${kind}`}</button>
            </article>
          ))}
        </div>
        <div className="resource-installed">
          <div className="design-panel-label">Installed {resourceLabel} ({installed.length})</div>
          {installed.length === 0 ? <div className="workspace-tool-empty">Nothing installed yet.</div> : installed.map((item) => <div className="resource-installed-row mono" key={`${item.target}:${item.path}`}><span><strong>{item.name}</strong><small className={item.valid ? "resource-valid" : "resource-invalid"}>{item.valid ? "Ready" : "Needs review"} · {item.detail}</small></span><span className="resource-installed-target">{item.target}</span></div>)}
        </div>
        <div className="workspace-tool-output mono">{result}</div>
      </section>
    );
  }

  return (
    <section className="workspace-tool-screen" aria-label={`${screenLabel(screen)} workspace`}>
      <h2 className="mono">{screenLabel(screen)}</h2>
      <p className="workspace-tool-summary">Live project-backed tools for <span className="mono">{root}</span>.</p>
      <div className="workspace-tool-actions">
        <button className="toolbar-btn toolbar-btn-primary" type="button" onClick={() => void refresh()} disabled={loading}>
          {loading ? "Refreshing…" : "Refresh data"}
        </button>
        <span className="mono workspace-tool-count">{entries.length} root entries · {fileEntries.length} files</span>
      </div>
      <div className="workspace-tool-output mono">
        {result}
      </div>
      <div className="workspace-tool-files">
        <div className="workspace-tool-files-title mono">Open project resources</div>
        {fileEntries.length === 0 ? (
          <div className="workspace-tool-empty">No root files are available in this project.</div>
        ) : fileEntries.map((entry) => (
          <button className="workspace-tool-file mono" type="button" key={entry.path} onClick={() => onOpenFile(entry.path)} title={`Open ${entry.path}`}>
            {entry.name}
          </button>
        ))}
      </div>
    </section>
  );
}
