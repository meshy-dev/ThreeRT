#!/bin/sh
# Kaleido in a window, live: press M to switch between ThreeRT's denoiser and the best this device has (MetalFX's
# joint denoiser on a Mac), N to step through every method (builtin, metalfx-denoised, metalfx, fsr, off). The
# window's title says which ran, the frame rate and the render scale. Esc quits.
# The window is 960 x 540 points: 1920 x 1080 pixels on a Retina display, the recordings' size.
#   tools/denoise-demo/live.sh [extra play flags, e.g. --quality balanced or --width 1280 --height 720]
cd "$(dirname "$0")/../.." || exit 1
[ -f out/bundles/kaleido.js ] || node tools/bundle-game.ts games/kaleido/kaleido.game.ts -o out/bundles/kaleido.js
# (The radiance cache over-brightens with hundreds of moving lights: off, as in the recordings.)
THREERT_CACHE=0 exec renderer/target/release/threert-render play out/bundles/kaleido.js --assets . --presets presets \
  --width 960 --height 540 --quality performance --reconstruction builtin "$@"
