// A small, dependency-free, stable string hash for the plugin's local echo gate.
//
// This does NOT need to match the server's content_hash. It only needs to be
// self-consistent so the plugin can tell whether a note body changed between syncs
// (a real user edit) versus matching what the graph last wrote (an echo to skip).

/** FNV-1a 32-bit hash, returned as zero-padded hex. */
export function localHash(input: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i);
    // hash *= 16777619, kept in 32-bit range via Math.imul.
    hash = Math.imul(hash, 0x01000193);
  }
  // Coerce to unsigned and hex-encode.
  return (hash >>> 0).toString(16).padStart(8, "0");
}
