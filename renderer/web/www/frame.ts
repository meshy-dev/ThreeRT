// A frame's numbers as the page records them (main.ts), apart from the page so a tool can name the type
// (tools/web-hello.ts).

/** A frame's numbers (ms): the page's own times around the module's (`frameEnd`'s JSON). */
export interface FrameRow {
  /** Since the frame before (the page's frame callback). */
  interval: number;
  /** `frameBegin`, the node host's `frame` (of which the engine's calls: `calls` of them, `calls_ms`), `frameEnd`. */
  begin: number;
  script: number;
  calls: number;
  calls_ms: number;
  end: number;
  physics: { ticks: number; step: number; sync: number; bodies: number; moved: number } | null;
  physics_ms: number;
  commit_ms: number;
  rebuilt: boolean;
  rebuild_ms: number;
  render_ms: number;
  gpu_ms: number;
  trace_ms: number;
  scale: number;
  frame: number;
  /** The kernel's invocations the loop guard has stopped since the view opened (0: every frame is sound). */
  guard_trips: number | null;
}
