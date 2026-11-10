// World paths: "state/<json path>" (the world is the game state; see store.ts).
// Segments are escaped like JSON pointers (~0 = "~", ~1 = "/"), so "/" always separates segments.

export function escapeSegment(s: string): string {
  return s.replace(/~/g, "~0").replace(/\//g, "~1");
}

export function unescapeSegment(s: string): string {
  return s.replace(/~1/g, "/").replace(/~0/g, "~");
}

export function joinPath(...segments: (string | number)[]): string {
  return segments.map((s) => escapeSegment(String(s))).join("/");
}

/** Appends one (unescaped) key to an already escaped path. */
export function childPath(path: string, key: string | number): string {
  return `${path}/${escapeSegment(String(key))}`;
}

export function splitPath(path: string): string[] {
  return path.split("/").map(unescapeSegment);
}

/** `a` is `b` or one of its ancestors. */
export function covers(a: string, b: string): boolean {
  return a === b || b.startsWith(a + "/");
}

/** "a/b/c" -> ["a", "a/b"] */
export function strictAncestors(path: string): string[] {
  const out: string[] = [];
  for (let i = path.indexOf("/"); i >= 0; i = path.indexOf("/", i + 1)) out.push(path.slice(0, i));
  return out;
}

/** "a/b/c" -> ["a", "a/b", "a/b/c"] */
export function prefixes(path: string): string[] {
  return [...strictAncestors(path), path];
}
