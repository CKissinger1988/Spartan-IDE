import React, { useCallback, useEffect, useState } from "react";
import { listDirectory, type FsEntry } from "../fsAccess";

interface FileTreeProps {
  root: FileSystemDirectoryHandle;
  onOpenFile: (handle: FileSystemFileHandle, path: string) => void;
}

/** One real, lazily-expanded directory node -- children are listed via a
 * real File System Access API call the first time it's expanded, then
 * cached. Mirrors `desktop/src/components/FileTree.tsx`'s own real
 * lazy-expansion design (itself mirroring the original wgpu shell's
 * `file_tree.rs`, §75.26) -- same shape, a real browser directory handle
 * in place of an IPC round trip. */
function TreeNode({
  entry,
  path,
  depth,
  onOpenFile,
}: {
  entry: FsEntry;
  path: string;
  depth: number;
  onOpenFile: (handle: FileSystemFileHandle, path: string) => void;
}): React.ReactElement {
  const [expanded, setExpanded] = useState(false);
  const [children, setChildren] = useState<FsEntry[] | null>(null);

  const toggle = useCallback(async () => {
    if (entry.kind === "file") {
      onOpenFile(entry.handle as FileSystemFileHandle, path);
      return;
    }
    if (!expanded && children === null) {
      const listed = await listDirectory(entry.handle as FileSystemDirectoryHandle);
      setChildren(listed);
    }
    setExpanded((e) => !e);
  }, [entry, expanded, children, onOpenFile, path]);

  return (
    <div>
      <div className="tree-row" style={{ paddingLeft: 8 + depth * 14 }} onClick={toggle}>
        {entry.kind === "directory" ? (
          <span className="tree-caret">{expanded ? "v" : ">"}</span>
        ) : (
          <span className="tree-caret" />
        )}
        <span className="mono">{entry.name}</span>
      </div>
      {entry.kind === "directory" && expanded && children && (
        <div>
          {children.map((child) => (
            <TreeNode
              key={child.name}
              entry={child}
              path={`${path}/${child.name}`}
              depth={depth + 1}
              onOpenFile={onOpenFile}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export default function FileTree({ root, onOpenFile }: FileTreeProps): React.ReactElement {
  const [rootEntries, setRootEntries] = useState<FsEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    listDirectory(root)
      .then((entries) => {
        if (!cancelled) setRootEntries(entries);
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
        <TreeNode
          key={entry.name}
          entry={entry}
          path={entry.name}
          depth={0}
          onOpenFile={onOpenFile}
        />
      ))}
    </div>
  );
}
