// Finds a Chrome (or Chromium) binary: CHROME when set, else the macOS app bundle, else the usual names on PATH
// (google-chrome, google-chrome-stable, chromium, chromium-browser: how Linux distributions install it).
import { existsSync } from "node:fs";
import { delimiter, join } from "node:path";

const MAC_APP = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PATH_NAMES = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"];

export interface ChromeLookup {
  env?: Record<string, string | undefined>;
  platform?: string;
  exists?: (path: string) => boolean;
}

/** The Chrome binary to launch, or null when there is none. CHROME is returned as given, without a check. */
export function findChrome({ env = process.env, platform = process.platform, exists = existsSync }: ChromeLookup = {}): string | null {
  if (env.CHROME) return env.CHROME;
  if (platform === "darwin" && exists(MAC_APP)) return MAC_APP;
  for (const dir of (env.PATH ?? "").split(delimiter).filter(Boolean))
    for (const name of PATH_NAMES) if (exists(join(dir, name))) return join(dir, name);
  return null;
}

/** findChrome, or the macOS path for an error message that names what was looked for. */
export const chromeOrDefault = (lookup?: ChromeLookup) => findChrome(lookup) ?? MAC_APP;
