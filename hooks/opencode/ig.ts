/**
 * ig-rewrite plugin for OpenCode.
 *
 * Rewrites grep/rg/find/cat shell calls to their ig equivalents.
 * Loaded by adding `import "./ig.ts"` to your opencode entry, or by
 * `opencode.json` instructions pointing at this file.
 */

export const id = "ig-rewrite";
export const version = "1.0.0";

export function rewrite(cmd: string): string {
  const s = cmd.trim();
  if (s.startsWith("grep ") || s.startsWith("rg ")) {
    return "ig " + s.slice(s.indexOf(" ") + 1);
  }
  if (s.startsWith("find ") && s.includes(" -name ")) {
    return "ig files";
  }
  if (s.startsWith("cat ")) {
    return "ig read " + s.slice(4);
  }
  return cmd;
}
