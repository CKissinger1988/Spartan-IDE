import React, { useCallback, useEffect, useState } from "react";
import type { BackendClient } from "../backendClient";

interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
}

interface BackendFileTreeProps {
  client: BackendClient;
  root: string;
  onOpenFile: (path: string) => void;
}

/**
 * Real, backend-rooted file tree -- a direct port of
 * `desktop/src/components/FileTree.tsx` onto `BackendClient.call` (same
 * substitution `GitPanel.tsx` already made), listing files under the
 * connected devserver's own real project root via real `list_dir` IPC
 * calls, lazily per directory expansion.
 *
 * Deliberately separate from the existing `FileTree.tsx` (File System
 * Access API-backed) rather than a unified component: the two have
 * genuinely different data sources (a real OS path string here vs. a
 * `FileSystemDirectoryHandle` there) and different real capabilities
 * (only this one can drive `BackendEditor`'s real LSP diagnostics).
 */
function TreeNode({
  client,
  entry,
  depth,
  onOpenFile,
}: {
  client: BackendClient;
  entry: Entry;
  depth: number;
  onOpenFile: (path: string) => void;
}): React.ReactElement {
  const [expanded, setExpanded] = useState(false);
  const [children, setChildren] = useState<Entry[] | null>(null);

  const toggle = useCallback(async () => {
    if (!entry.is_dir) {
      onOpenFile(entry.path);
      return;
    }
    if (!expanded && children === null) {
      const result = (await client.call("list_dir", { path: entry.path })) as {
        entries: Entry[];
      };
      setChildren(result.entries);
    }
    setExpanded((e) => !e);
  }, [client, entry, expanded, children, onOpenFile]);

  return (
    <div>
      <div className="tree-row" style={{ paddingLeft: 8 + depth * 14 }} onClick={toggle}>
        {entry.is_dir ? (
          <span className="tree-caret">{expanded ? "v" : ">"}</span>
        ) : (
          <span className="tree-caret" />
        )}
        <span className="mono">{entry.name}</span>
      </div>
      {entry.is_dir && expanded && children && (
        <div>
          {children.map((child) => (
            <TreeNode key={child.path} client={client} entry={child} depth={depth + 1} onOpenFile={onOpenFile} />
          ))}
        </div>
      )}
    </div>
  );
}

export default function BackendFileTree({
  client,
  root,
  onOpenFile,
}: BackendFileTreeProps): React.ReactElement {
  const [rootEntries, setRootEntries] = useState<Entry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    client
      .call("list_dir", { path: root })
      .then((result) => {
        if (!cancelled) setRootEntries((result as { entries: Entry[] }).entries);
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [client, root]);

  return (
    <div className="file-tree">
      {error && <div className="tree-error">{error}</div>}
      {rootEntries?.map((entry) => (
        <TreeNode key={entry.path} client={client} entry={entry} depth={0} onOpenFile={onOpenFile} />
      ))}
    </div>
  );
}
