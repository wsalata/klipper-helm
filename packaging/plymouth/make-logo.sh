#!/bin/sh
# Renders logo.png for the Plymouth boot splash from the panel itself, so the splash and the
# panel's welcome screen ("connecting") match pixel for pixel. Needs a desktop session (the
# panel opens a window) and ImageMagick.
#
# The crop is the logo and the "klipperhelm" wordmark of NotReadyPanel (ui/dialogs.slint) at
# scale 2: logical x 260..540, y 126..362 on an 800x480 window. klipper-helm.script places it
# at the same spot; change both if that screen's layout changes.
set -e
cd "$(dirname "$0")/../.."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cargo build --quiet
# An address that never answers keeps the panel on its "connecting" welcome screen
SLINT_SCALE_FACTOR=2 KLIPPER_HELM_CONFIG="$TMP/none.toml" \
	MOONRAKER_URL=ws://10.255.255.1:7125/websocket KLIPPER_HELM_SNAPSHOT="$TMP" \
	timeout 60 ./target/debug/klipper-helm >/dev/null 2>&1 || true
magick "$TMP/page-0.ppm" -crop 560x472+520+252 +repage packaging/plymouth/logo.png
magick identify packaging/plymouth/logo.png
