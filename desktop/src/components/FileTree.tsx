import React, { useCallback, useEffect, useState } from "react";

interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
}

interface FileTreeProps {
  root: string;
  onOpenFile: (path: string) => void;
}

/** One real, lazily-expanded directory node -- children are fetched via
 * a real `list_dir` IPC call the first time it's expanded, then cached,
 * mirroring the original wgpu shell's own `file_tree.rs` lazy-expansion
 * design (§75.26), just moved across the IPC boundary instead of an
 * in-process call. */
function TreeNode({
  entry,
  depth,
  onOpenFile,
}: {
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
      const result = (await window.spartan.call("list_dir", { path: entry.path })) as {
        entries: Entry[];
      };
      setChildren(result.entries);
    }
    setExpanded((e) => !e);
  }, [entry, expanded, children, onOpenFile]);

  return (
    <div>
      <div
        className="tree-row"
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={toggle}
      >
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
            <TreeNode key={child.path} entry={child} depth={depth + 1} onOpenFile={onOpenFile} />
          ))}
        </div>
      )}
    </div>
  );
}

export default function FileTree({ root, onOpenFile }: FileTreeProps): React.ReactElement {
  const [rootEntries, setRootEntries] = useState<Entry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    window.spartan
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
  }, [root]);

  return (
    <div className="file-tree">
      {error && <div className="tree-error">{error}</div>}
      {rootEntries?.map((entry) => (
        <TreeNode key={entry.path} entry={entry} depth={0} onOpenFile={onOpenFile} />
      ))}
    </div>
  );
}
