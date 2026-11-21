import { convertFileSrc } from "@tauri-apps/api/core";

let base: string | undefined;

/**
 * URL of a `media://` resource. The scheme differs per platform (`http://media.localhost/` on
 * Windows, `media://localhost/` on Linux); `convertFileSrc` knows which one applies.
 */
export function mediaUrl(path: string): string {
  base ??= convertFileSrc("", "media");
  return base + path.replace(/^\//, "");
}
