// A converted city area as the scene sees it (games/tokyo/areas/<area>.ts, written by tools/tokyo/to-glb.ts).
export type AreaIndex = {
  area: string;
  origin: { lon: number; lat: number };
  tileSize: number;
  bounds: { minX: number; minZ: number; maxX: number; maxZ: number };
  /** A digest of the area's GLBs (tools/tokyo/to-glb.ts), so the bundle changes when they do. */
  build: string;
  counts: { tiles: number; triangles: number; buildings: number; windows: number; litWindows: number; props: Record<string, number>; signs: number; roadSymbols: number; images: number; glbBytes: number };
  tiles: { x: number; z: number; file: string }[];
};

/**
 * Lit windows switch on by building, in groups, as the sun goes down: tools/tokyo/to-glb.ts writes each group's lit
 * windows to a GLB node of its own (`WINDOW_NODES`), and a time-of-day scene switches each group's power at
 * `windowSwitchAt(g)`, the sun's elevation in degrees. Offices come on from late afternoon (12 to 0 degrees), shops round
 * sunset (6 to -2), homes into the dusk (4 to -8), each building at its own moment within its span.
 */
export const WINDOW_GROUPS = 6;
export const WINDOW_SWITCH = { high: 12, low: -8 };
export const WINDOW_NODES = Array.from({ length: WINDOW_GROUPS }, (_, g) => `win${g}`);
export const windowSwitchAt = (g: number) => WINDOW_SWITCH.high - ((g + 0.5) * (WINDOW_SWITCH.high - WINDOW_SWITCH.low)) / WINDOW_GROUPS;
