/** Characters Windows does not allow in file names, plus control characters. */
// eslint-disable-next-line no-control-regex -- control characters are exactly what we replace
const INVALID_FILE_NAME_CHARS = /[<>:"/\\|?*\u0000-\u001f]/g;

/**
 * A file name stem made from a display name (a project name): characters invalid on Windows become
 * `_`, trailing dots and spaces are dropped (Windows strips them). `fallback` when nothing is left.
 */
export function safeFileStem(name: string, fallback: string): string {
  const stem = name
    .replace(INVALID_FILE_NAME_CHARS, "_")
    .trim()
    .replace(/[. ]+$/, "");
  return stem || fallback;
}

/** The last segment of a Windows or POSIX path: `C:\a\b.mp4` → `b.mp4`. */
export function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut < 0 ? path : path.slice(cut + 1);
}
