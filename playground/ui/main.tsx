// ThreeRT Playground UI (PocketJS / Solid): Unreal-Editor-style layout around the native real-time viewport.
//   main toolbar (Play / Stop, undo, A/B vs UE5) · viewport (compositor surface, overlays)
//   outliner · details (scrub fields edit the live scene) · content browser · output log · acceptance · game
import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { mount } from "@pocketjs/framework/solid";
import { CompositorSurface, Text, View } from "@pocketjs/framework/components";
import { app, cmd, connect, host, log, onWheel, overlay, stats, type CatalogItem, type NodeDetails, type Row, type WorldDetails } from "./bridge.ts";
import { Badge, Button, Check, detailRows, Divider, Dropdown, fmt, HLine, Icon, named, PanelHeader, PropRow, ReadOnly, Scrub, Section, Segmented, Swatch, TabBar, ToolButton, Vec3Field } from "./widgets.tsx";

const ICON: Record<string, string> = {
  camera: "icons/camera.svg", sun: "icons/sun.svg", pointlight: "icons/pointlight.svg", spotlight: "icons/spotlight.svg",
  rectlight: "icons/rectlight.svg", mesh: "icons/mesh.svg", model: "icons/model.svg", group: "icons/group.svg",
};
const KIND_LABEL: Record<string, string> = {
  camera: "CineCamera", sun: "Directional Light", pointlight: "Point Light", spotlight: "Spot Light", rectlight: "Rect Light",
  mesh: "Static Mesh", model: "glTF Asset", group: "Group",
};
const VIEW_MODES = ["Lit", "Base Color", "World Normal", "Lit (no denoiser)", "Path Tracing (raw)", "Temporal History"];
// Option lists are module constants: a list rebuilt inside a reactive expression re-creates its rows on every stats
// update (4 Hz), which resets the hover highlight.
const VIEW_MODE_ITEMS = VIEW_MODES.map((l, i) => ({ label: l, value: i, hint: `Alt+${i + 1}` }));
const BOUNCE_ITEMS = [{ label: "Auto (up to 4, holds 60 fps)", value: 0, hint: "default" }, ...[1, 2, 3, 4, 6, 8].map((b) => ({ label: `${b} bounce${b > 1 ? "s" : ""}`, value: b, hint: "fixed" }))];
const SPP_ITEMS = [{ label: "Adaptive (fill 75% of frame)", value: 16, hint: "" }, ...[1, 2, 4, 8].map((n) => ({ label: `${n} spp / frame`, value: n, hint: "" }))];
const COMPARE_OPTIONS = [
  { value: "live", label: "ThreeRT Live" },
  { value: "split-lumen", label: "UE5 Lumen" },
  { value: "split-pathtracer", label: "UE5 Path Tracer" },
  { value: "split-three", label: "three.js" },
];
const playing = () => !!app.game?.playing;
const set = (path: string, value: unknown) => cmd("set", { path, value });

// ------------------------------------------------------------------------------------------------ title + toolbar

function Toolbar() {
  const scenes = createMemo(() => app.catalog.map((c) => ({ label: c.title, value: c.id, hint: c.kind === "game" ? "game" : `${c.triangles} tris` })));
  const st = stats;
  const hasGame = () => app.scene?.kind === "game";
  const refAvailable = (v: string) => (v === "split-lumen" ? st().refs[0] : v === "split-pathtracer" ? st().refs[1] : v === "split-three" ? !!(st().refs as boolean[])[2] : true);
  return (
    <View class="relative z-40 h-[42px] w-full flex-row items-center gap-[4px] px-[8px] bg-[#242424]">
      <View class="flex-row items-center pl-[6px] pr-[6px]">
        <Text class="text-sm text-[#e8e8e8] font-bold">ThreeRT Playground</Text>
      </View>
      <Divider />
      <Dropdown id="scene" icon={app.scene?.kind === "game" ? "icons/game.svg" : "icons/level.svg"} label={`${app.scene?.title ?? "Open scene"}${(app.scene?.edits ?? 0) > 0 ? " *" : ""}`} items={scenes()} onSelect={(id) => cmd("open", { id })} width={230} />
      <Divider />
      <ToolButton name="play" icon={hasGame() && !playing() ? "icons/play.svg" : "icons/play-dim.svg"} label="Play" disabled={!hasGame() || playing()} onPress={() => cmd("play")} />
      <ToolButton name="stop" icon={playing() ? "icons/stop.svg" : "icons/stop-dim.svg"} label="Stop" disabled={!playing()} onPress={() => cmd("stop")} />
      <Divider />
      <ToolButton name="undo" icon={app.scene?.canUndo ? "icons/undo.svg" : "icons/undo-dim.svg"} disabled={!app.scene?.canUndo} onPress={() => cmd("undo")} />
      <ToolButton name="redo" icon={app.scene?.canRedo ? "icons/redo.svg" : "icons/redo-dim.svg"} disabled={!app.scene?.canRedo} onPress={() => cmd("redo")} />
      <Show when={(app.scene?.edits ?? 0) > 0}>
        <ToolButton label="Revert" onPress={() => cmd("revert")} />
      </Show>
      <View class="flex-1" />
      <Text class="text-2xs text-[#8c8c8c]">COMPARE</Text>
      <Segmented name="compare" value={st().compare} onChange={(mode) => cmd("compare", { mode })} options={COMPARE_OPTIONS} enabled={refAvailable} />
      <Divider />
      <ToolButton icon="icons/fullscreen.svg" onPress={() => cmd("fullscreen")} />
    </View>
  );
}

// ------------------------------------------------------------------------------------------------ viewport

function StatLine(p: { label: string; value: string; tone?: "good" | "warn" | "bad" }) {
  return (
    <View class="h-[15px] flex-row items-center gap-[8px]" style={{ hitPass: 1 }}>
      <View class="w-[64px]" style={{ hitPass: 1 }}><Text class="text-2xs text-[#9a9a9a]" style={{ hitPass: 1 }}>{p.label}</Text></View>
      <Text class={p.tone === "bad" ? "text-xs text-[#ff6060] font-mono" : p.tone === "warn" ? "text-xs text-[#f0c040] font-mono" : "text-xs text-[#7cfc7c] font-mono"} style={{ hitPass: 1 }}>{p.value}</Text>
    </View>
  );
}

function Viewport() {
  const st = stats;
  const budget = () => 1000 / host().fps;
  const tone = (ms: number) => (ms < budget() * 0.9 ? "good" : ms < budget() * 1.2 ? "warn" : "bad");
  const conv = () => ((st().history ?? 0) >= 4096 ? "converged" : `${fmt(st().history ?? 0, 0)} frames`);
  const split = () => st().compare.startsWith("split");
  const refName = () => (st().compare.endsWith("pathtracer") ? "UE5 Path Tracer (offline)" : st().compare === "split-three" ? "three.js r187 (same scene code)" : "UE5 Lumen (offline)");
  return (
    <View class="flex-1 w-full p-[2px] bg-[#0a0a0a]">
      <CompositorSurface package="viewport" class="w-full h-full bg-[#000000]" nodeRef={(node: any) => overlay.control("viewport", node)} />
      {/* viewport toolbar (UE5: overlaid on the viewport) */}
      <View class="absolute top-[8px] left-[8px] flex-row items-center gap-[4px]">
        <Dropdown id="viewmode" overlay label={VIEW_MODES[st().mode ?? 0]} items={VIEW_MODE_ITEMS} onSelect={(mode) => cmd("viewMode", { mode })} width={210} />
        <Dropdown id="bounces" overlay label={st().autoBounces === false ? `Bounces ${st().maxBounces}` : `Bounces ${st().bounces ?? 4} (auto)`} items={BOUNCE_ITEMS} onSelect={(value) => cmd("bounces", { value })} width={240} />
        <Dropdown id="spp" overlay label={st().sppCap >= 16 ? "Samples: Adaptive" : `Samples: ${st().sppCap}`} items={SPP_ITEMS} onSelect={(value) => cmd("sppCap", { value })} width={220} />
      </View>
      <View class="absolute top-[8px] right-[10px] flex-row items-center gap-[4px]">
        <View focusable onPress={() => cmd("homeCamera")} class={st().sceneCamera ? "h-[24px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#0070e0] transition duration-200 ease-out active:scale-95" : "h-[24px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#151515e0] focus:bg-[#383838] transition duration-200 ease-out active:scale-95"}>
          <Icon src="icons/camera.svg" size={14} />
          <Text class="text-xs text-[#e0e0e0]">Scene Camera</Text>
        </View>
        <View focusable onPress={() => cmd("focus")} class="h-[24px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#151515e0] focus:bg-[#383838] transition duration-150 ease-out active:scale-95">
          <Icon src="icons/focus.svg" size={14} />
          <Text class="text-xs text-[#e0e0e0]">Focus</Text>
        </View>
        <View focusable onPress={() => cmd("outline")} class={st().outline ? "h-[24px] flex-row items-center px-[8px] rounded-[4px] bg-[#151515e0] focus:bg-[#383838] transition duration-200 ease-out active:scale-95" : "h-[24px] flex-row items-center px-[8px] rounded-[4px] bg-[#6b4d10] focus:bg-[#383838] transition duration-200 ease-out active:scale-95"}>
          <Text class="text-xs text-[#e0e0e0]">{st().outline ? "Outline" : "Game View"}</Text>
        </View>
      </View>
      {/* stat unit */}
      <View class="absolute top-[42px] right-[12px] flex-col px-[10px] py-[6px] rounded-[4px] bg-[#000000a8] animate-fade" style={{ hitPass: 1 }}>
        <StatLine label="FPS" value={fmt(st().fps, 1)} tone={st().fps >= host().fps * 0.95 ? "good" : st().fps >= host().fps * 0.8 ? "warn" : "bad"} />
        <StatLine label="GPU" value={`${fmt(st().gpu_ms ?? 0, 2)} ms`} tone={tone(st().gpu_ms ?? 0)} />
        <StatLine label="Frame CPU" value={`${fmt(st().cpuMs, 2)} ms`} tone={tone(st().cpuMs)} />
        <StatLine label="UI" value={`${fmt(st().uiMs, 2)} ms`} />
        <StatLine label="Samples" value={`${st().spp} spp · ${st().bounces ?? "-"} bnc`} />
        <StatLine label="Temporal" value={conv()} tone={(st().history ?? 0) >= 16 ? "good" : "warn"} />
        <StatLine label="Render" value={`${st().width ?? 0}x${st().height ?? 0} · ${Math.round((st().scale ?? 1) * 100)}%`} />
        <StatLine label="Triangles" value={`${st().triangles ?? 0}`} />
      </View>
      <Show when={split()}>
        <View class="absolute bottom-[34px] left-[12px] px-[8px] py-[4px] rounded-[4px] bg-[#000000b0] animate-rise" style={{ hitPass: 1 }}>
          <Text class="text-xs text-[#ffffff] font-bold" style={{ hitPass: 1 }}>ThreeRT real-time (60 fps)</Text>
        </View>
        <View class="absolute bottom-[34px] right-[12px] px-[8px] py-[4px] rounded-[4px] bg-[#0b4a8fd8] animate-rise" style={{ hitPass: 1 }}>
          <Text class="text-xs text-[#ffffff] font-bold" style={{ hitPass: 1 }}>{refName()}</Text>
        </View>
      </Show>
      <Show when={playing()}>
        <View class="absolute top-[42px] left-[10px] flex-row items-center gap-[8px] px-[12px] h-[24px] rounded-[4px] bg-[#1f5c2a] animate-drop" style={{ hitPass: 1 }}>
          <View class="animate-breathe"><Icon src="icons/dot-green.svg" size={12} /></View>
          <Text class="text-xs text-[#ffffff] font-bold" style={{ hitPass: 1 }}>{`PLAYING  ·  seq ${app.game?.seq ?? 0}  ·  click to interact, Esc to stop`}</Text>
        </View>
      </Show>
      <View class="absolute bottom-[8px] left-[12px]" style={{ hitPass: 1 }}>
        <Text class="text-2xs text-[#9a9a9a]" style={{ hitPass: 1 }}>
          {st().compare.startsWith("split") ? "Drag to move the split  ·  ThreeRT Live to fly again" : "LMB orbit  ·  RMB look + WASD/QE fly  ·  MMB/Shift pan  ·  wheel dolly  ·  click select  ·  F focus  ·  Home scene camera"}
        </Text>
      </View>
    </View>
  );
}

// ------------------------------------------------------------------------------------------------ outliner + details

function OutlinerRow(p: { row: Row; index: number }) {
  const sel = () => app.selected === p.row.id;
  return (
    <View
      focusable
      nodeRef={named(`row:${p.row.id}`)}
      onPress={() => cmd("select", { node: p.row.id })}
      class={sel() ? "h-[22px] w-full flex-row items-center gap-[6px] pr-[8px] bg-[#0070e0] transition-colors duration-150 ease-out" : p.index % 2 ? "h-[22px] w-full flex-row items-center gap-[6px] pr-[8px] bg-[#1d1d1d] focus:bg-[#2f2f2f] transition-colors duration-150 ease-out" : "h-[22px] w-full flex-row items-center gap-[6px] pr-[8px] bg-[#1a1a1a] focus:bg-[#2f2f2f] transition-colors duration-150 ease-out"}
    >
      <View style={{ width: 10 + p.row.depth * 14 }} />
      <Icon src={ICON[p.row.kind] ?? "icons/group.svg"} size={14} />
      <View class="flex-1"><Text class={p.row.visible ? "text-xs text-[#dadada]" : "text-xs text-[#6a6a6a]"}>{p.row.name}</Text></View>
      <View class="w-[110px]"><Text class={sel() ? "text-2xs text-[#e0eeff]" : "text-2xs text-[#7c7c7c]"}>{p.row.active ? "Active Camera" : KIND_LABEL[p.row.kind] ?? p.row.kind}</Text></View>
      <View focusable onPress={() => cmd("set", { node: p.row.id, path: "node.visible", value: !p.row.visible })} class="w-[18px] h-[18px] items-center justify-center rounded-[3px] bg-[#1a1a1a01] focus:bg-[#3a3a3a] transition duration-150 ease-out active:scale-90">
        <Icon src={p.row.visible ? "icons/eye.svg" : "icons/eye-off.svg"} size={14} />
      </View>
    </View>
  );
}

const OUTLINER_H = 300;
const ROW_H = 22;

function Outliner() {
  const [top, setTop] = createSignal(0);
  const rows = () => app.outliner;
  const visible = Math.floor((OUTLINER_H - 30 - 22) / ROW_H) - 1; // minus the World row
  const maxTop = () => Math.max(0, rows().length - visible);
  const first = () => Math.min(top(), maxTop());
  let acc = 0;
  return (
    <View class="h-[300px] w-full flex-col bg-[#1a1a1a]">
      <PanelHeader title="Outliner" icon="icons/level.svg" right={<Text class="text-2xs text-[#7c7c7c] pr-[8px] pb-[8px]">{`${rows().length} actors`}</Text>} />
      <View class="h-[22px] w-full flex-row items-center px-[10px] bg-[#242424]">
        <View class="flex-1"><Text class="text-2xs text-[#9a9a9a] font-bold">ITEM LABEL</Text></View>
        <View class="w-[134px]"><Text class="text-2xs text-[#9a9a9a] font-bold">TYPE</Text></View>
      </View>
      <View
        class="flex-1 w-full flex-row overflow-hidden bg-[#1a1a1a]"
        nodeRef={(n: any) => onCleanup(onWheel(n, (dy) => {
          acc -= dy;
          const steps = Math.trunc(acc / ROW_H);
          acc -= steps * ROW_H;
          setTop(Math.max(0, Math.min(maxTop(), first() + steps)));
        }))}
      >
        <View class="flex-1 flex-col">
          <View focusable onPress={() => cmd("select", { node: null })} class={app.selected === null ? "h-[22px] w-full flex-row items-center gap-[6px] px-[10px] bg-[#0070e0] transition-colors duration-150 ease-out" : "h-[22px] w-full flex-row items-center gap-[6px] px-[10px] bg-[#1a1a1a] focus:bg-[#2f2f2f] transition-colors duration-150 ease-out"}>
            <Icon src="icons/world.svg" size={14} />
            <View class="flex-1"><Text class="text-xs text-[#dadada]">{app.scene ? `${app.scene.id} (World)` : "World"}</Text></View>
            <View class="w-[128px]"><Text class="text-2xs text-[#7c7c7c]">World Settings</Text></View>
          </View>
          <For each={rows().slice(first(), first() + visible)}>{(row) => <OutlinerRow row={row} index={rows().indexOf(row)} />}</For>
        </View>
        <Show when={maxTop() > 0}>
          <View class="w-[6px] h-full flex-col bg-[#151515]">
            <View style={{ height: Math.round((OUTLINER_H - 52) * (first() / rows().length)) }} />
            <View class="w-full rounded-[3px] bg-[#4a4a4a]" style={{ height: Math.max(16, Math.round((OUTLINER_H - 52) * (visible / rows().length))) }} />
          </View>
        </Show>
      </View>
    </View>
  );
}

function NodeDetailsView(p: { n: NodeDetails }) {
  const n = () => p.n;
  const ro = () => playing();
  return (
    <>
      <View class="h-[46px] w-full flex-row items-center gap-[10px] px-[10px] bg-[#1a1a1a]">
        <Icon src={ICON[n().kind] ?? "icons/group.svg"} size={22} />
        <View class="flex-1 flex-col">
          <Text class="text-sm text-[#ffffff] font-bold">{n().name}</Text>
          <Text class="text-2xs text-[#8c8c8c]">{`${KIND_LABEL[n().kind] ?? n().kind}  ·  id ${n().id}${n().parent ? `  ·  parent ${n().parent}` : ""}`}</Text>
        </View>
        <Show when={ro()}><Badge text="PLAY (read-only)" tone="green" /></Show>
      </View>
      <HLine />
      <Section title="Transform">
        <PropRow label="Location"><Vec3Field name="scrub:node.location" value={n().location} step={0.005} onChange={(i, v) => set(`node.location.${i}`, v)} /></PropRow>
        <PropRow label="Rotation"><Vec3Field name="scrub:node.rotation" value={n().rotation} step={0.5} digits={1} onChange={(i, v) => set(`node.rotation.${i}`, v)} /></PropRow>
        <PropRow label="Scale"><Vec3Field value={n().scale} step={0.005} onChange={(i, v) => set(`node.scale.${i}`, v)} /></PropRow>
      </Section>
      <Show when={n().material}>
        {(m) => (
          <Section title="Material" right={<Text class="text-2xs text-[#8c8c8c]">{`${m().name}${m().users > 1 ? `  ·  shared by ${m().users}` : ""}`}</Text>}>
            <PropRow label="Base Color"><Swatch rgb={m().color} /><Vec3Field value={m().color} step={0.003} min={0} max={1} colors={false} onChange={(i, v) => set(`material.color.${i}`, v)} /></PropRow>
            <PropRow label="Roughness"><Scrub name="scrub:material.roughness" value={m().roughness} step={0.004} min={0} max={1} onChange={(v) => set("material.roughness", v)} /></PropRow>
            <PropRow label="Metallic"><Scrub name="scrub:material.metalness" value={m().metalness} step={0.004} min={0} max={1} onChange={(v) => set("material.metalness", v)} /></PropRow>
            <PropRow label="Emissive"><Swatch rgb={m().emissive} /><Vec3Field value={m().emissive} step={0.004} min={0} colors={false} onChange={(i, v) => set(`material.emissive.${i}`, v)} /></PropRow>
            <PropRow label="Emissive (nits)"><Scrub value={m().emissiveIntensity} step={0.01} mul min={0} digits={2} onChange={(v) => set("material.emissiveIntensity", v < 1e-3 ? 0 : v)} /></PropRow>
            <PropRow label="Opacity"><Scrub value={m().opacity} step={0.004} min={0} max={1} onChange={(v) => set("material.opacity", v)} /></PropRow>
          </Section>
        )}
      </Show>
      <Show when={n().light}>
        {(l) => (
          <Section title="Light" right={<Text class="text-2xs text-[#8c8c8c]">{`${l().type}  ·  photometric (${l().unit})`}</Text>}>
            <PropRow label={`Intensity (${l().unit})`}><Scrub name="scrub:light.intensity" value={l().intensity} step={0.008} mul min={0} digits={1} onChange={(v) => set("light.intensity", v)} /></PropRow>
            <PropRow label="Light Color"><Swatch rgb={l().color} /><Vec3Field value={l().color} step={0.003} min={0} max={1} colors={false} onChange={(i, v) => set(`light.color.${i}`, v)} /></PropRow>
            <Show when={l().radius !== null}><PropRow label="Source Radius"><Scrub value={l().radius!} step={0.002} min={0} onChange={(v) => set("light.radius", v)} /></PropRow></Show>
            <Show when={l().angle !== null}><PropRow label="Outer Cone (deg)"><Scrub value={l().angle!} step={0.2} digits={1} min={1} max={89} onChange={(v) => set("light.angle", v)} /></PropRow></Show>
            <Show when={l().penumbra !== null}><PropRow label="Penumbra"><Scrub value={l().penumbra!} step={0.004} min={0} max={1} onChange={(v) => set("light.penumbra", v)} /></PropRow></Show>
            <Show when={l().width !== null}><PropRow label="Width / Height"><Scrub value={l().width!} step={0.005} min={0.01} onChange={(v) => set("light.width", v)} /><Scrub value={l().height!} step={0.005} min={0.01} onChange={(v) => set("light.height", v)} /></PropRow></Show>
            <Show when={l().angularDiameter !== null}><PropRow label="Source Angle (deg)"><Scrub value={l().angularDiameter!} step={0.01} min={0} max={20} onChange={(v) => set("light.angularDiameter", v)} /></PropRow></Show>
            <PropRow label="Cast Shadows"><Check value={l().castShadow} onChange={(v) => set("light.castShadow", v)} /></PropRow>
          </Section>
        )}
      </Show>
      <Show when={n().camera}>
        {(c) => (
          <Section title="Camera">
            <PropRow label="Field of View"><Scrub value={c().fov} step={0.1} digits={1} min={5} max={150} onChange={(v) => set("camera.fov", v)} /></PropRow>
            <Show when={c().exposure !== null}><PropRow label="Exposure (EV100)"><Scrub value={c().exposure!} step={0.01} digits={2} onChange={(v) => set("camera.exposure", v)} /></PropRow></Show>
            <PropRow label="Active Camera"><ReadOnly text={c().active ? "yes (scene camera, UE5 references use it)" : "no"} /></PropRow>
          </Section>
        )}
      </Show>
      <Section title="Rendering">
        <PropRow label="Visible"><Check name="check:node.visible" value={n().visible} onChange={(v) => set("node.visible", v)} /></PropRow>
        <PropRow label="Cast Shadow"><Check value={n().castShadow} onChange={(v) => set("node.castShadow", v)} /></PropRow>
        <Show when={n().geometry}><PropRow label="Geometry"><ReadOnly text={`${n().geometry!.triangles} triangles  ·  ${n().geometry!.id.slice(0, 18)}`} /></PropRow></Show>
        <Show when={n().model}><PropRow label="glTF Asset"><ReadOnly text={n().model!} /></PropRow></Show>
        <Show when={n().tags.length}><PropRow label="Gameplay Tags"><ReadOnly text={n().tags.join(", ")} /></PropRow></Show>
      </Section>
    </>
  );
}

function WorldDetailsView(p: { w: WorldDetails }) {
  const w = () => p.w;
  return (
    <>
      <View class="h-[46px] w-full flex-row items-center gap-[10px] px-[10px] bg-[#1a1a1a]">
        <Icon src="icons/world.svg" size={22} />
        <View class="flex-1 flex-col">
          <Text class="text-sm text-[#ffffff] font-bold">World Settings</Text>
          <Text class="text-2xs text-[#8c8c8c]">{`${w().name}  ·  preset ${w().preset}  ·  ${w().resolution[0]}x${w().resolution[1]}`}</Text>
        </View>
      </View>
      <HLine />
      <Section title="Sky & Lighting">
        <PropRow label="Sky"><ReadOnly text={w().sky === "atmosphere" ? `Sky Atmosphere (sun: ${w().sun ?? "none"})` : w().sky === "color" ? "Uniform sky color" : w().sky === "gradient" ? "Night sky gradient" : "HDRI"} /></PropRow>
        <PropRow label="Sky Light"><Scrub value={w().skyLight} step={0.004} min={0} onChange={(v) => set("world.skyLight", v)} /></PropRow>
        <Show when={w().skyIntensity !== null}><PropRow label="Sky (nits)"><Scrub value={w().skyIntensity!} step={0.008} mul min={0} digits={2} onChange={(v) => set("world.skyIntensity", v)} /></PropRow></Show>
        <Show when={w().skyColor}><PropRow label="Sky Color"><Swatch rgb={w().skyColor!} /><Vec3Field value={w().skyColor!} step={0.003} min={0} max={1} colors={false} onChange={(i, v) => set(`world.skyColor.${i}`, v)} /></PropRow></Show>
      </Section>
      <Show when={w().fog}>
        <Section title="Exponential Height Fog">
          <PropRow label="Density (1/m)"><Scrub value={w().fog!.density} step={0.008} mul min={0} digits={4} onChange={(v) => set("world.fogDensity", v)} /></PropRow>
          <PropRow label="Height Falloff"><Scrub value={w().fog!.heightFalloff} step={0.004} min={0} onChange={(v) => set("world.fogFalloff", v)} /></PropRow>
          <Show when={w().fog!.scattering !== null}><PropRow label="Volumetric"><Scrub value={w().fog!.scattering!} step={0.004} min={0} onChange={(v) => set("world.fogScattering", v)} /></PropRow></Show>
        </Section>
      </Show>
      <Section title="Weather">
        <PropRow label="Wetness"><Scrub value={w().wetness} step={0.004} min={0} max={1} onChange={(v) => set("world.wetness", v)} /></PropRow>
        <PropRow label="Rain"><Scrub value={w().rain} step={0.004} min={0} max={1} onChange={(v) => set("world.rain", v)} /></PropRow>
      </Section>
      <Section title="Engine">
        <PropRow label="Nodes"><ReadOnly text={`${w().counts.nodes}`} /></PropRow>
        <PropRow label="Materials"><ReadOnly text={`${w().counts.materials}`} /></PropRow>
        <PropRow label="Geometries"><ReadOnly text={`${w().counts.geometries}`} /></PropRow>
        <PropRow label="Textures"><ReadOnly text={`${w().counts.textures}`} /></PropRow>
      </Section>
    </>
  );
}

function Details() {
  const [offset, setOffset] = createSignal(0);
  // Visible height: window minus title bar, toolbar, outliner, splitters, panel tab and status bar.
  const viewH = () => (stats().win?.[1] ?? 900) - 42 - 1 - OUTLINER_H - 3 - 30 - 1 - 26;
  const maxOffset = () => Math.max(0, 47 + detailRows() * 26 + 24 - viewH());
  const off = () => Math.min(offset(), maxOffset());
  return (
    <View class="flex-1 w-full flex-col bg-[#1a1a1a] overflow-hidden" nodeRef={(n: any) => onCleanup(onWheel(n, (dy) => setOffset(Math.max(0, Math.min(maxOffset(), off() - dy)))))}>
      <PanelHeader title="Details" icon="icons/log.svg" right={<Show when={app.scene?.undoLabel}><Text class="text-2xs text-[#7c7c7c] pr-[8px] pb-[8px]">{`last edit: ${app.scene?.undoLabel}`}</Text></Show>} />
      <View class="w-full flex-col overflow-hidden" style={{ height: viewH() }}>
        <View style={{ marginT: -off() }} />
        <Show when={app.selected ?? "world"} keyed>
          {() => (
            <View class="w-full flex-col animate-rise">
              <Show when={app.details?.node} fallback={<Show when={app.details?.world}>{(w) => <WorldDetailsView w={w()} />}</Show>}>
                {(n) => <NodeDetailsView n={n()} />}
              </Show>
            </View>
          )}
        </Show>
      </View>
    </View>
  );
}

// ------------------------------------------------------------------------------------------------ bottom dock

const TILE = 150;
const TILE_GAP = 10;
const DOCK_H = 262;

function Tile(p: { c: CatalogItem }) {
  const c = p.c;
  const open = () => app.scene?.id === c.id;
  return (
    <View
      focusable
      nodeRef={named(`tile:${c.id}`)}
      onPress={() => cmd("open", { id: c.id })}
      class={open() ? "w-[150px] h-[150px] flex-col rounded-[4px] bg-[#2a2a2a] border-[2] border-[#0070e0] overflow-hidden transition duration-200 ease-out focus:translate-y-[-2px] active:scale-97" : "w-[150px] h-[150px] flex-col rounded-[4px] bg-[#242424] border-[2] border-[#242424] focus:border-[#575757] focus:bg-[#2a2a2a] overflow-hidden transition duration-200 ease-out focus:translate-y-[-2px] active:scale-97"}
    >
      <View class="w-full h-[82px] bg-[#0a0a0a]">
        <Show when={host().thumbs.includes(c.id)} fallback={<View class="w-full h-full items-center justify-center"><Icon src={c.kind === "game" ? "icons/game.svg" : "icons/level.svg"} size={28} /></View>}>
          <CompositorSurface package={`thumb:${c.id}`} class="w-full h-full" style={{ hitPass: 1 }} />
        </Show>
        <View class="absolute top-[4px] left-[4px]" style={{ hitPass: 1 }}>
          <Show when={c.verdict === "accepted"}><Badge text="ACCEPTED" tone="green" /></Show>
          <Show when={c.verdict === "rejected"}><Badge text="REJECTED" tone="red" /></Show>
        </View>
        <View class="absolute top-[4px] right-[4px]" style={{ hitPass: 1 }}>
          <Show when={c.kind === "game"} fallback={<Badge text="SCENE" tone="grey" />}><Badge text="GAME" tone="blue" /></Show>
        </View>
      </View>
      <View class="w-full h-[3px]" style={{ bgColor: c.kind === "game" ? "#9a7be0" : "#c9a24e" }} />
      <View class="flex-col px-[8px] py-[5px] gap-[2px]">
        <Text class="text-xs text-[#ffffff] font-bold">{c.title}</Text>
        <Text class="text-2xs text-[#9a9a9a]">{`${c.nodes} nodes  ·  ${c.triangles} tris`}</Text>
        <Text class="text-2xs text-[#9a9a9a]">{c.vsLumen !== null ? `dE vs Lumen ${fmt(c.vsLumen, 2)}` : c.refs.lumen ? "UE5 refs: Lumen + PT" : "no UE5 reference"}</Text>
      </View>
    </View>
  );
}

/** Scene grid: whole rows of fixed-size tiles (columns from the dock width), scrolled by the wheel. */
function ContentBrowser() {
  const [offset, setOffset] = createSignal(0);
  // Dock width: window minus the right column (400) and splitter (3), minus padding.
  // A memo (compared by value) so the 4 Hz stats updates do not rebuild the rows and reset tile hover.
  const cols = createMemo(() => Math.max(1, Math.floor(((stats().win?.[0] ?? 1400) - 403 - 20 + TILE_GAP) / (TILE + TILE_GAP))));
  const rows = createMemo(() => {
    const out: CatalogItem[][] = [];
    for (let i = 0; i < app.catalog.length; i += cols()) out.push(app.catalog.slice(i, i + cols()));
    return out;
  });
  const viewH = DOCK_H - 30 - 2 * TILE_GAP;
  const contentH = () => rows().length * (TILE + TILE_GAP) - TILE_GAP;
  const maxOffset = () => Math.max(0, contentH() - viewH);
  const off = () => Math.min(offset(), maxOffset());
  return (
    <View class="w-full flex-row overflow-hidden bg-[#1a1a1a] animate-fade" style={{ height: DOCK_H - 30 }} nodeRef={(n: any) => onCleanup(onWheel(n, (dy) => setOffset(Math.max(0, Math.min(maxOffset(), off() - dy)))))}>
      <View class="flex-1 h-full flex-col gap-[10px] p-[10px] overflow-hidden">
        <View class="shrink-0" style={{ marginT: -off() }} />
        <For each={rows()}>{(row) => <View class="flex-row shrink-0 gap-[10px]"><For each={row}>{(c) => <Tile c={c} />}</For></View>}</For>
      </View>
      <Show when={maxOffset() > 0}>
        <View class="w-[6px] h-full flex-col py-[10px] bg-[#151515]">
          <View style={{ height: Math.round(viewH * (off() / contentH())) }} />
          <View class="w-full rounded-[3px] bg-[#4a4a4a]" style={{ height: Math.max(16, Math.round(viewH * (viewH / contentH()))) }} />
        </View>
      </Show>
    </View>
  );
}

function OutputLog() {
  const lines = createMemo(() => log().slice(-12));
  return (
    <View class="flex-1 w-full flex-col px-[10px] py-[6px] gap-[1px] overflow-hidden bg-[#121212] animate-fade">
      <For each={lines()}>
        {(l) => (
          <View class="h-[17px] w-full flex-row items-center gap-[8px] animate-rise">
            <View class="w-[86px] shrink-0"><Text class="text-2xs text-[#6c6c6c]">{`Log${l.source[0].toUpperCase()}${l.source.slice(1)}`}</Text></View>
            <Text class={l.level === "error" ? "text-xs text-[#ff6464] font-mono" : l.level === "warn" ? "text-xs text-[#e8c050] font-mono" : l.level === "tx" ? "text-xs text-[#86d886] font-mono" : "text-xs text-[#bdbdbd] font-mono"}>{l.text}</Text>
          </View>
        )}
      </For>
    </View>
  );
}

function Acceptance() {
  const s = () => app.scene;
  const st = stats;
  return (
    <View class="flex-1 w-full flex-row gap-[12px] p-[10px] overflow-hidden animate-fade">
      <View class="w-[300px] flex-col gap-[6px]">
        <Text class="text-sm text-[#ffffff] font-bold">{s()?.title ?? "-"}</Text>
        <Text class="text-2xs text-[#9a9a9a]">{s()?.description ?? ""}</Text>
        <View class="h-[6px]" />
        <Text class="text-2xs text-[#8c8c8c] font-bold">COMPARE WITH UE5 (pixel-aligned, scene camera)</Text>
        <View class="flex-row gap-[6px]">
          <Button label="UE5 Lumen" icon="icons/split.svg" disabled={!st().refs[0]} onPress={() => cmd("compare", { mode: "split-lumen" })} />
          <Button label="UE5 Path Tracer" icon="icons/split.svg" disabled={!st().refs[1]} onPress={() => cmd("compare", { mode: "split-pathtracer" })} />
        </View>
        <View class="flex-row gap-[6px]">
          <Button label="three.js (same code)" icon="icons/split.svg" disabled={!(st().refs as boolean[])[2]} onPress={() => cmd("compare", { mode: "split-three" })} />
        </View>
        <View class="flex-row gap-[6px]">
          <Button label="Live" onPress={() => cmd("compare", { mode: "live" })} />
          <Button label="Reset history" onPress={() => cmd("resetHistory")} />
        </View>
        <View class="h-[6px]" />
        <Text class="text-2xs text-[#8c8c8c] font-bold">VERDICT</Text>
        <View class="flex-row gap-[6px] items-center">
          <Button name="btn:accept" label="Accept" icon="icons/check.svg" primary onPress={() => cmd("verdict", { value: "accepted" })} />
          <Button label="Reject" icon="icons/cross.svg" onPress={() => cmd("verdict", { value: "rejected" })} />
          <Button name="btn:clear" label="Clear" onPress={() => cmd("verdict", { value: null })} />
          <Show when={s()?.verdict}>
            {(v) => <Badge text={v().verdict === "accepted" ? "ACCEPTED" : "REJECTED"} tone={v().verdict === "accepted" ? "green" : "red"} />}
          </Show>
        </View>
      </View>
      <View class="flex-1 flex-col gap-[4px]">
        <Text class="text-2xs text-[#8c8c8c] font-bold">MEASURED (tools/bench-quality.ts, tools/bench-live.ts; 1280x720, M5)</Text>
        <View class="h-[20px] flex-row items-center px-[6px] bg-[#242424]">
          <View class="w-[150px]"><Text class="text-2xs text-[#9a9a9a] font-bold">FRAMES OF HISTORY</Text></View>
          <View class="w-[80px]"><Text class="text-2xs text-[#9a9a9a] font-bold">GPU MS</Text></View>
          <View class="w-[110px]"><Text class="text-2xs text-[#9a9a9a] font-bold">dE VS LUMEN</Text></View>
          <View class="w-[110px]"><Text class="text-2xs text-[#9a9a9a] font-bold">dE VS PT</Text></View>
          <View class="w-[110px]"><Text class="text-2xs text-[#9a9a9a] font-bold">SSIM VS LUMEN</Text></View>
        </View>
        <For each={s()?.realtime ?? []}>
          {(r) => (
            <View class="h-[18px] flex-row items-center px-[6px]">
              <View class="w-[150px]"><Text class="text-xs text-[#d0d0d0] font-mono">{r.frames === 1 ? "1 (disocclusion)" : r.frames === 256 ? "256 (still, ~4 s)" : `${r.frames}`}</Text></View>
              <View class="w-[80px]"><Text class="text-xs text-[#d0d0d0] font-mono">{fmt(r.gpuMs, 1)}</Text></View>
              <View class="w-[110px]"><Text class="text-xs text-[#d0d0d0] font-mono">{fmt(r.vsLumen, 2)}</Text></View>
              <View class="w-[110px]"><Text class="text-xs text-[#d0d0d0] font-mono">{fmt(r.vsPt, 2)}</Text></View>
              <View class="w-[110px]"><Text class="text-xs text-[#d0d0d0] font-mono">{fmt(r.ssimLumen, 3)}</Text></View>
            </View>
          )}
        </For>
        <Show when={!(s()?.realtime?.length)}><Text class="text-xs text-[#7c7c7c]">no real-time benchmark rows for this scene yet (node tools/bench-live.ts)</Text></Show>
        <View class="h-[4px]" />
        <Text class="text-xs text-[#b0b0b0]">{`UE5 Lumen ${fmt(s()?.ue.lumen_s ?? null, 1)} s / frame  ·  UE5 Path Tracer ${fmt(s()?.ue.pathtracer_s ?? null, 1)} s / frame  ·  Lumen vs PT dE ${fmt(s()?.ue.lumen_vs_pt?.mean_delta_e ?? null, 2)}`}</Text>
        <Text class="text-xs text-[#7cfc7c]">{`ThreeRT now: ${fmt(st().fps, 1)} fps  ·  GPU ${fmt(st().gpu_ms ?? 0, 2)} ms  ·  ${(st().history ?? 0) >= 4096 ? "converged" : `history ${fmt(st().history ?? 0, 0)}`}`}</Text>
      </View>
      <View class="w-[230px] flex-col gap-[3px]">
        <Text class="text-2xs text-[#8c8c8c] font-bold">ACCEPTANCE CHECKLIST</Text>
        <For each={app.catalog}>
          {(c) => (
            <View focusable onPress={() => cmd("open", { id: c.id })} class={app.scene?.id === c.id ? "h-[20px] flex-row items-center gap-[6px] px-[6px] rounded-[3px] bg-[#2f2f2f] transition-colors duration-150 ease-out" : "h-[20px] flex-row items-center gap-[6px] px-[6px] rounded-[3px] bg-[#1a1a1a] focus:bg-[#2a2a2a] transition-colors duration-150 ease-out"}>
              <Icon src={c.verdict === "accepted" ? "icons/dot-green.svg" : c.verdict === "rejected" ? "icons/dot-red.svg" : "icons/dot-grey.svg"} size={12} />
              <View class="flex-1"><Text class="text-xs text-[#d0d0d0]">{c.title}</Text></View>
              <Text class="text-2xs text-[#8c8c8c]">{c.verdict ?? "pending"}</Text>
            </View>
          )}
        </For>
      </View>
    </View>
  );
}

const CONTROL_PAYLOADS: Record<string, { label: string; payload: Record<string, unknown> }[]> = {
  lights: [{ label: "Lights", payload: {} }],
  orbit: [{ label: "Orbit -", payload: { dir: -1 } }, { label: "Orbit +", payload: { dir: 1 } }],
  zoom: [{ label: "Zoom -", payload: { dir: -1 } }, { label: "Zoom +", payload: { dir: 1 } }],
};

function GamePanel() {
  const g = () => app.game;
  return (
    <Show when={g()} fallback={<View class="flex-1 p-[12px]"><Text class="text-xs text-[#8c8c8c]">This scene has no gameplay. Open a game (Kabukichō Night, Light Room: Three-like scene + Verse-style transactional handlers) to Play.</Text></View>}>
      <View class="flex-1 w-full flex-row gap-[12px] p-[10px] overflow-hidden animate-fade">
        <View class="w-[250px] flex-col gap-[6px]">
          <View class="flex-row gap-[6px]">
            <Button label={g()!.playing ? "Stop" : "Play"} icon={g()!.playing ? "icons/stop.svg" : "icons/play.svg"} primary={!g()!.playing} onPress={() => cmd(g()!.playing ? "stop" : "play")} />
            <Show when={g()!.playing}><Badge text={`SEQ ${g()!.seq}`} tone="blue" /></Show>
          </View>
          <Text class="text-2xs text-[#8c8c8c] font-bold">HANDLERS (tx.decide / tx.attempt, optimistic commit)</Text>
          <View class="flex-row flex-wrap gap-[4px]">
            <For each={g()!.handlers.filter((h) => h !== "click")}>
              {(h) => <For each={CONTROL_PAYLOADS[h] ?? [{ label: h[0].toUpperCase() + h.slice(1), payload: {} }]}>{(c) => <Button label={c.label} disabled={!g()!.playing} onPress={() => cmd("gameEvent", { event: h, payload: c.payload, label: `${h} ${JSON.stringify(c.payload)}` })} />}</For>}
            </For>
          </View>
          <Text class="text-2xs text-[#8c8c8c]">{`events: ${g()!.handlers.join(", ")}${g()!.tick ? ", tick (30 Hz)" : ""}`}</Text>
          <Text class="text-2xs text-[#8c8c8c]">In Play, viewport clicks become click events on the object under the cursor; L / R / V trigger lights, rain and viewpoint when the game has them.</Text>
        </View>
        <View class="flex-1 flex-col gap-[1px]">
          <View class="h-[20px] flex-row items-center px-[6px] bg-[#242424]">
            <View class="w-[50px]"><Text class="text-2xs text-[#9a9a9a] font-bold">SEQ</Text></View>
            <View class="w-[190px]"><Text class="text-2xs text-[#9a9a9a] font-bold">EVENT</Text></View>
            <View class="w-[70px]"><Text class="text-2xs text-[#9a9a9a] font-bold">STATUS</Text></View>
            <View class="w-[70px]"><Text class="text-2xs text-[#9a9a9a] font-bold">READ/WRITE</Text></View>
            <View class="w-[60px]"><Text class="text-2xs text-[#9a9a9a] font-bold">MS</Text></View>
            <View class="flex-1"><Text class="text-2xs text-[#9a9a9a] font-bold">NOTE</Text></View>
          </View>
          <For each={g()!.tx.slice(0, 9)}>
            {(t) => (
              <View class="h-[17px] flex-row items-center px-[6px]">
                <View class="w-[50px]"><Text class="text-xs text-[#d0d0d0] font-mono">{`${t.seq}`}</Text></View>
                <View class="w-[190px]"><Text class="text-xs text-[#d0d0d0] font-mono">{t.event}</Text></View>
                <View class="w-[70px]"><Text class={t.status === "ok" ? "text-xs text-[#86d886] font-mono" : "text-xs text-[#e8c050] font-mono"}>{t.status}</Text></View>
                <View class="w-[70px]"><Text class="text-xs text-[#d0d0d0] font-mono">{`${t.reads}/${t.writes}`}</Text></View>
                <View class="w-[60px]"><Text class="text-xs text-[#d0d0d0] font-mono">{fmt(t.ms, 2)}</Text></View>
                <View class="flex-1"><Text class="text-2xs text-[#9a9a9a]">{t.note}</Text></View>
              </View>
            )}
          </For>
        </View>
        <View class="w-[260px] flex-col px-[8px] py-[4px] rounded-[4px] bg-[#0f0f0f] overflow-hidden">
          <Text class="text-2xs text-[#8c8c8c] font-bold">GAME STATE (tx.state)</Text>
          <For each={g()!.state.slice(0, 11)}>{(l) => <Text class="text-xs text-[#b8d8ff] font-mono">{l}</Text>}</For>
        </View>
      </View>
    </Show>
  );
}

type Dock = "content" | "log" | "acceptance" | "game";

function BottomDock(p: { tab: Dock; setTab: (t: Dock) => void }) {
  return (
    <View class="w-full flex-col bg-[#1a1a1a]" style={{ height: DOCK_H }}>
      <TabBar
        value={p.tab}
        onChange={p.setTab}
        tabs={[
          { value: "content", label: "Content Browser", icon: "icons/grid.svg" },
          { value: "log", label: "Output Log", icon: "icons/log.svg" },
          { value: "acceptance", label: "Acceptance", icon: "icons/check.svg" },
          { value: "game", label: "Game", icon: "icons/game.svg" },
        ]}
      />
      <Show when={p.tab === "content"}><ContentBrowser /></Show>
      <Show when={p.tab === "log"}><OutputLog /></Show>
      <Show when={p.tab === "acceptance"}><Acceptance /></Show>
      <Show when={p.tab === "game"}><GamePanel /></Show>
    </View>
  );
}

function StatusBar(p: { setTab: (t: Dock) => void }) {
  const st = stats;
  const last = createMemo(() => log().at(-1));
  return (
    <View class="h-[26px] w-full flex-row items-center gap-[4px] px-[6px] bg-[#151515]">
      <View focusable onPress={() => p.setTab("content")} class="h-[22px] flex-row items-center gap-[6px] px-[8px] rounded-[3px] bg-[#15151501] focus:bg-[#2a2a2a] transition-colors duration-150 ease-out">
        <Icon src="icons/grid.svg" size={12} /><Text class="text-2xs text-[#b0b0b0]">Content Drawer</Text>
      </View>
      <View focusable onPress={() => p.setTab("log")} class="h-[22px] flex-row items-center gap-[6px] px-[8px] rounded-[3px] bg-[#15151501] focus:bg-[#2a2a2a] transition-colors duration-150 ease-out">
        <Icon src="icons/log.svg" size={12} /><Text class="text-2xs text-[#b0b0b0]">Output Log</Text>
      </View>
      <Divider />
      <View class="flex-1 overflow-hidden"><Text class={last()?.level === "error" ? "text-2xs text-[#ff6464]" : last()?.level === "warn" ? "text-2xs text-[#e8c050]" : "text-2xs text-[#8a8a8a]"}>{last()?.text ?? ""}</Text></View>
      <Text class="text-2xs text-[#8a8a8a]">{`${fmt(st().fps, 0)} fps  ·  GPU ${fmt(st().gpu_ms ?? 0, 1)} ms  ·  reload ${fmt(st().reloadMs, 0)} ms`}</Text>
    </View>
  );
}

function App() {
  connect();
  const [tab, setTab] = createSignal<Dock>("content");
  return (
    <View class="w-full h-full flex-col bg-[#151515]">
      <Toolbar />
      <HLine />
      <View class="flex-1 w-full flex-row">
        <View class="flex-1 h-full flex-col">
          <Viewport />
          <View class="h-[3px] w-full bg-[#0a0a0a]" />
          <BottomDock tab={tab()} setTab={setTab} />
        </View>
        <View class="w-[3px] h-full bg-[#0a0a0a]" />
        <View class="w-[400px] h-full flex-col">
          <Outliner />
          <View class="h-[3px] w-full bg-[#0a0a0a]" />
          <Details />
        </View>
      </View>
      <HLine />
      <StatusBar setTab={setTab} />
    </View>
  );
}

mount(() => <App />);
