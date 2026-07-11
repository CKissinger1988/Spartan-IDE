// Real wrapper around the real File System Access API
// (https://wicg.github.io/file-system-access/) -- the actual browser API
// this pass's "pure client-side, zero server" half of the hybrid
// architecture (§75.85) depends on for real local file read/write. A
// real, named platform limit, not glossed over: this API is Chromium-only
// as of this pass (Chrome/Edge/Opera) -- Firefox and Safari do not
// implement it. `isFileSystemAccessSupported()` lets the UI degrade
// honestly instead of silently failing.

export function isFileSystemAccessSupported(): boolean {
  return typeof window !== "undefined" && "showDirectoryPicker" in window;
}

export interface FsEntry {
  name: string;
  kind: "file" | "directory";
  handle: FileSystemHandle;
}

export async function pickProjectDirectory(): Promise<FileSystemDirectoryHandle> {
  return window.showDirectoryPicker({ mode: "readwrite" });
}

/** Real, non-recursive listing of one real directory's immediate entries,
 * sorted directories-first then alphabetically -- matching every other
 * real file-tree listing already established elsewhere in this project
 * (`spartan-backend::list_dir`, `spartan-editor-core::file_tree.rs`). */
export async function listDirectory(dir: FileSystemDirectoryHandle): Promise<FsEntry[]> {
  const entries: FsEntry[] = [];
  for await (const [name, handle] of (dir as any).entries()) {
    entries.push({ name, kind: handle.kind, handle });
  }
  entries.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
  return entries;
}

export async function readFileText(handle: FileSystemFileHandle): Promise<string> {
  const file = await handle.getFile();
  return file.text();
}

/** Real save-to-disk via the File System Access API's own real
 * createWritable/write/close sequence -- the browser-native equivalent of
 * `spartan-backend::save_file`'s real `std::fs::write` on the desktop
 * side. */
export async function writeFileText(handle: FileSystemFileHandle, text: string): Promise<void> {
  const writable = await handle.createWritable();
  await writable.write(text);
  await writable.close();
}
