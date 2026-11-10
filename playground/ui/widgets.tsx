// Unreal-Editor-style controls built from PocketJS primitives. Class strings are whole literals (PocketJS compiles
// them ahead of time); per-instance colors go through `style`.
//
// Motion (one vocabulary for every control; timelines in theme.json):
//   hover / press   `transition duration-150 ease-out` on colors and transforms, `active:scale-95` on buttons
//   selection       segmented thumb slides (transform transition), chevrons rotate, check marks spring in
//   appear          popups `animate-menu`, panels `animate-fade`, content `animate-rise`, banners `animate-drop`
import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { Image, Text, View } from "@pocketjs/framework/components";
import { overlay } from "./bridge.ts";

/** Registers a control under a semantic name (automation / UI tests address controls by it). */
export const named = (name?: string) => (n: any) => {
  if (name) onCleanup(overlay.control(name, n));
};

export const C = {
  recessed: "#0f0f0f",
  background: "#151515",
  panel: "#1a1a1a",
  header: "#242424",
  button: "#383838",
  hover: "#575757",
  accent: "#0070e0",
  text: "#c0c0c0",
  bright: "#ffffff",
  dim: "#808080",
  x: "#c4433c",
  y: "#5fa83a",
  z: "#3a72c8",
};

/** Linear Rec.709 -> sRGB hex (for swatches of scene colors). */
export function srgbHex(rgb: readonly number[]): string {
  const ch = (c: number) => {
    const v = Math.max(0, Math.min(1, c));
    const s = v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;
    return Math.round(s * 255).toString(16).padStart(2, "0");
  };
  return `#${ch(rgb[0])}${ch(rgb[1])}${ch(rgb[2])}`;
}

export function fmt(v: number | null | undefined, digits = 3): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "-";
  const a = Math.abs(v);
  if (a >= 10000) return v.toFixed(0);
  if (a >= 100) return v.toFixed(Math.min(digits, 1));
  return v.toFixed(digits);
}

export function Icon(p: { src: string; size?: number }) {
  return <Image src={p.src} style={{ width: p.size ?? 16, height: p.size ?? 16 }} />;
}

export function Divider() {
  return <View class="w-[1px] h-[22px] mx-[4px] bg-[#3a3a3a]" />;
}

export function HLine() {
  return <View class="w-full h-[1px] bg-[#0f0f0f]" />;
}

/** Main-toolbar button: icon and/or label on the toolbar background, lighter under the pointer. */
export function ToolButton(p: { icon?: string; label?: string; onPress: () => void; disabled?: boolean; active?: boolean; name?: string }) {
  return (
    <View
      focusable
      nodeRef={named(p.name)}
      onPress={() => { if (!p.disabled) p.onPress(); }}
      class={p.active ? "h-[28px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#0070e0] transition duration-150 ease-out active:scale-95" : p.disabled ? "h-[28px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#242424] transition duration-150 ease-out" : "h-[28px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#242424] focus:bg-[#383838] active:bg-[#0f0f0f] transition duration-150 ease-out active:scale-95"}
    >
      <Show when={p.icon}><Icon src={p.icon!} /></Show>
      <Show when={p.label}>
        <Text class={p.disabled ? "text-xs text-[#5c5c5c] transition-colors duration-150" : p.active ? "text-xs text-[#ffffff] transition-colors duration-150" : "text-xs text-[#c0c0c0] transition-colors duration-150"}>{p.label}</Text>
      </Show>
    </View>
  );
}

/** Filled button (panels, dialogs). */
export function Button(p: { label: string; icon?: string; onPress: () => void; primary?: boolean; disabled?: boolean; name?: string }) {
  return (
    <View
      focusable
      nodeRef={named(p.name)}
      onPress={() => { if (!p.disabled) p.onPress(); }}
      class={p.primary ? "h-[24px] flex-row items-center gap-[6px] px-[10px] rounded-[4px] bg-[#0070e0] focus:bg-[#1a86f0] active:bg-[#005bb8] transition duration-150 ease-out active:scale-95" : "h-[24px] flex-row items-center gap-[6px] px-[10px] rounded-[4px] bg-[#383838] focus:bg-[#4a4a4a] active:bg-[#0070e0] transition duration-150 ease-out active:scale-95"}
    >
      <Show when={p.icon}><Icon src={p.icon!} size={14} /></Show>
      <Text class={p.disabled ? "text-xs text-[#5c5c5c]" : "text-xs text-[#e6e6e6]"}>{p.label}</Text>
    </View>
  );
}

/**
 * Segmented choice (UE toolbar toggles). Equal-width options over a thumb that slides to the selection.
 * `options` should be a constant list (rows keep their hover state).
 */
export function Segmented<T extends string>(p: { name?: string; value: T; options: { value: T; label: string }[]; width?: number; enabled?: (v: T) => boolean; onChange: (v: T) => void }) {
  const on = (v: T) => p.enabled?.(v) ?? true;
  const w = () => p.width ?? 116;
  const index = () => Math.max(0, p.options.findIndex((o) => o.value === p.value));
  return (
    <View class="relative h-[28px] flex-row items-center p-[2px] gap-[2px] rounded-[4px] bg-[#0f0f0f]">
      <View class="absolute top-[2px] left-[2px] h-[24px] rounded-[3px] bg-[#3a3a3a] transition-transform duration-200 ease-out" style={{ width: w(), translateX: index() * (w() + 2) }} />
      <For each={p.options}>
        {(o) => (
          <View
            focusable
            nodeRef={named(p.name && `${p.name}:${o.value}`)}
            onPress={() => { if (on(o.value)) p.onChange(o.value); }}
            class="h-[24px] flex-row items-center justify-center rounded-[3px] bg-[#0f0f0f01] focus:bg-[#ffffff14] active:bg-[#ffffff08] transition-colors duration-150 ease-out"
            style={{ width: w() }}
          >
            <Text class={!on(o.value) ? "text-xs text-[#4a4a4a] transition-colors duration-200" : p.value === o.value ? "text-xs text-[#ffffff] transition-colors duration-200" : "text-xs text-[#9a9a9a] transition-colors duration-200"}>{o.label}</Text>
          </View>
        )}
      </For>
    </View>
  );
}

const [openMenu, setOpenMenu] = createSignal<string | null>(null);
export const closeMenus = () => setOpenMenu(null);

/** Dropdown: a button with the current value and a popup list (one menu open at a time). */
export function Dropdown<T>(p: { id: string; label: string; icon?: string; items: { label: string; value: T; hint?: string }[]; onSelect: (v: T) => void; overlay?: boolean; width?: number }) {
  const open = () => openMenu() === p.id;
  return (
    <View class="relative">
      <View
        focusable
        nodeRef={named(`dd:${p.id}`)}
        onPress={() => setOpenMenu(open() ? null : p.id)}
        class={p.overlay ? (open() ? "h-[24px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#383838] transition duration-150 ease-out" : "h-[24px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#151515e0] focus:bg-[#383838] transition duration-150 ease-out") : open() ? "h-[28px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#383838] transition duration-150 ease-out" : "h-[28px] flex-row items-center gap-[6px] px-[8px] rounded-[4px] bg-[#242424] focus:bg-[#383838] transition duration-150 ease-out"}
      >
        <Show when={p.icon}><Icon src={p.icon!} size={14} /></Show>
        <Text class="text-xs text-[#d0d0d0]">{p.label}</Text>
        <View class="transition-transform duration-150 ease-out" style={{ rotate: open() ? 180 : 0 }}><Icon src="icons/chev-down.svg" size={12} /></View>
      </View>
      <Show when={open()}>
        <View class="absolute top-[30px] left-[0px] z-50 flex-col py-[4px] rounded-[4px] bg-[#242424] border border-[#0a0a0a] shadow-lg animate-menu origin-top-left" style={{ minW: p.width ?? 170 }}>
          <For each={p.items}>
            {(it, i) => (
              <View
                focusable
                nodeRef={named(`dd:${p.id}:${i()}`)}
                onPress={() => { setOpenMenu(null); p.onSelect(it.value); }}
                class="h-[24px] flex-row items-center justify-between gap-[16px] px-[12px] bg-[#24242401] focus:bg-[#0070e0] transition-colors duration-100 ease-out"
              >
                <Text class="text-xs text-[#d8d8d8]">{it.label}</Text>
                <Show when={it.hint}><Text class="text-2xs text-[#8a8a8a]">{it.hint}</Text></Show>
              </View>
            )}
          </For>
        </View>
      </Show>
    </View>
  );
}

export function TabBar<T extends string>(p: { tabs: { value: T; label: string; icon?: string }[]; value: T; onChange: (v: T) => void; right?: JSX.Element }) {
  return (
    <View class="h-[30px] w-full flex-row items-end bg-[#151515] px-[4px]">
      <For each={p.tabs}>
        {(t) => (
          <View
            focusable
            nodeRef={named(`tab:${t.value}`)}
            onPress={() => p.onChange(t.value)}
            class={p.value === t.value ? "h-[28px] flex-row items-center gap-[6px] px-[12px] rounded-[4px] bg-[#1a1a1a] transition-colors duration-200 ease-out" : "h-[28px] flex-row items-center gap-[6px] px-[12px] rounded-[4px] bg-[#151515] focus:bg-[#202020] transition-colors duration-200 ease-out"}
          >
            <Show when={t.icon}><Icon src={t.icon!} size={14} /></Show>
            <Text class={p.value === t.value ? "text-xs text-[#ffffff] transition-colors duration-200" : "text-xs text-[#8c8c8c] transition-colors duration-200"}>{t.label}</Text>
          </View>
        )}
      </For>
      <View class="flex-1" />
      {p.right}
    </View>
  );
}

/** Panel tab header (UE dock tab with the panel name). */
export function PanelHeader(p: { title: string; icon?: string; right?: JSX.Element }) {
  return (
    <View class="h-[30px] w-full flex-row items-end bg-[#151515] px-[4px]">
      <View class="h-[28px] flex-row items-center gap-[6px] px-[12px] rounded-[4px] bg-[#1a1a1a]">
        <Show when={p.icon}><Icon src={p.icon!} size={14} /></Show>
        <Text class="text-xs text-[#ffffff]">{p.title}</Text>
      </View>
      <View class="flex-1" />
      {p.right}
    </View>
  );
}

/** Collapsible Details category. */
export function Section(p: { title: string; children: JSX.Element; right?: JSX.Element }) {
  const [open, setOpen] = createSignal(true);
  countRow();
  return (
    <View class="w-full flex-col">
      <View focusable onPress={() => setOpen(!open())} class="h-[26px] w-full flex-row items-center gap-[6px] px-[8px] bg-[#2a2a2a] focus:bg-[#323232] transition-colors duration-150 ease-out">
        <View class="transition-transform duration-150 ease-out" style={{ rotate: open() ? 0 : -90 }}><Icon src="icons/chev-down.svg" size={12} /></View>
        <Text class="text-xs text-[#e8e8e8] font-bold">{p.title}</Text>
        <View class="flex-1" />
        {p.right}
      </View>
      <Show when={open()}>
        <View class="w-full flex-col py-[3px] bg-[#1a1a1a] animate-fade">{p.children}</View>
      </Show>
    </View>
  );
}

/** Mounted Details rows and section headers (26 px each): the scrollable extent of the Details panel. */
export const [detailRows, setDetailRows] = createSignal(0);
function countRow() {
  setDetailRows((n) => n + 1);
  onCleanup(() => setDetailRows((n) => n - 1));
}

export function PropRow(p: { label: string; children: JSX.Element }) {
  countRow();
  return (
    <View class="h-[26px] w-full flex-row items-center px-[10px] gap-[8px]">
      <View class="w-[104px] shrink-0"><Text class="text-xs text-[#b4b4b4]">{p.label}</Text></View>
      <View class="flex-1 flex-row items-center gap-[4px]">{p.children}</View>
    </View>
  );
}

export function ReadOnly(p: { text: string }) {
  return <Text class="text-xs text-[#8c8c8c]">{p.text}</Text>;
}

/**
 * Numeric field edited by dragging horizontally across it (UE "scrub" entry). `step` = value per logical pixel;
 * `mul` scrubs multiplicatively (intensities spanning decades).
 */
export function Scrub(p: { value: number; step: number; digits?: number; axis?: string; mul?: boolean; min?: number; max?: number; name?: string; onChange: (v: number) => void }) {
  let start = 0;
  const clamp = (v: number) => Math.min(p.max ?? Infinity, Math.max(p.min ?? -Infinity, v));
  return (
    <View
      class="flex-1 h-[20px] flex-row items-center rounded-[3px] bg-[#0f0f0f] overflow-hidden transition-colors duration-150 ease-out"
      nodeRef={(n: any) => {
        const dispose = overlay.drag(n, (d) => {
          if (d.started) { start = p.value; return; }
          p.onChange(clamp(p.mul ? start * Math.exp(d.dx * p.step) : start + d.dx * p.step));
        });
        onCleanup(dispose);
        named(p.name)(n);
      }}
    >
      <Show when={p.axis}><View class="w-[3px] h-full" style={{ bgColor: p.axis! }} /></Show>
      <View class="px-[6px]"><Text class="text-xs text-[#d0d0d0]">{fmt(p.value, p.digits ?? 3)}</Text></View>
    </View>
  );
}

export function Vec3Field(p: { value: readonly number[]; step: number; digits?: number; colors?: boolean; onChange: (i: number, v: number) => void; min?: number; max?: number; name?: string }) {
  const axes = [C.x, C.y, C.z];
  return (
    <>
      <For each={[0, 1, 2]}>
        {(i) => <Scrub name={p.name && `${p.name}.${i}`} value={p.value[i]} step={p.step} digits={p.digits} axis={p.colors === false ? undefined : axes[i]} min={p.min} max={p.max} onChange={(v) => p.onChange(i, v)} />}
      </For>
    </>
  );
}

export function Check(p: { value: boolean; onChange: (v: boolean) => void; name?: string }) {
  return (
    <View
      focusable
      nodeRef={named(p.name)}
      onPress={() => p.onChange(!p.value)}
      class={p.value ? "w-[16px] h-[16px] items-center justify-center rounded-[3px] bg-[#0070e0] border border-[#0070e0] focus:bg-[#1a86f0] transition duration-150 ease-out active:scale-90" : "w-[16px] h-[16px] items-center justify-center rounded-[3px] bg-[#0f0f0f] border border-[#4a4a4a] focus:bg-[#242424] transition duration-150 ease-out active:scale-90"}
    >
      <Show when={p.value}><View class="animate-tick"><Icon src="icons/tick.svg" size={12} /></View></Show>
    </View>
  );
}

export function Swatch(p: { rgb: readonly number[] }) {
  return <View class="w-[34px] h-[18px] shrink-0 rounded-[3px] border border-[#0a0a0a]" style={{ bgColor: srgbHex(p.rgb) }} />;
}

export function Badge(p: { text: string; tone: "green" | "red" | "blue" | "grey" | "amber" }) {
  return (
    <View class={p.tone === "green" ? "h-[18px] px-[6px] items-center justify-center rounded-[3px] bg-[#1f5c2a] animate-fade" : p.tone === "red" ? "h-[18px] px-[6px] items-center justify-center rounded-[3px] bg-[#6b2222] animate-fade" : p.tone === "blue" ? "h-[18px] px-[6px] items-center justify-center rounded-[3px] bg-[#0b4a8f] animate-fade" : p.tone === "amber" ? "h-[18px] px-[6px] items-center justify-center rounded-[3px] bg-[#6b4d10] animate-fade" : "h-[18px] px-[6px] items-center justify-center rounded-[3px] bg-[#333333] animate-fade"}>
      <Text class="text-2xs text-[#f0f0f0] font-bold">{p.text}</Text>
    </View>
  );
}
