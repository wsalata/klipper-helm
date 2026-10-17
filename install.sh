#!/bin/bash
# Installs Klipper Helm on the printer's computer next to Klipper, Moonraker and Mainsail
# (Raspberry Pi OS, MainsailOS or another Debian-based system), or removes it again:
#
#   curl -fsSL https://raw.githubusercontent.com/wsalata/klipper-helm/main/install.sh | bash
#   bash install.sh --uninstall
#
# Run it as the user Klipper runs as (not root); it asks for sudo where needed. Updates then
# come through Moonraker's update manager, like Klipper's and Mainsail's.
set -euo pipefail

REPO=wsalata/klipper-helm
DIR="$HOME/klipper-helm"
DATA="$HOME/printer_data"
CONFIG_DIR="$DATA/config"
MOONRAKER_CONF="$CONFIG_DIR/moonraker.conf"
ASVC="$DATA/moonraker.asvc"
SERVICE=/etc/systemd/system/klipper-helm.service
POLKIT_RULE=/etc/polkit-1/rules.d/50-klipper-helm.rules
STATE_DIR=/var/lib/klipper-helm
SECTION="[update_manager klipper-helm]"
USER=${USER:-$(id -un)}

say() { printf '\033[1;36m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*" >&2; }
die() {
	printf '\033[1;31m%s\033[0m\n' "$*" >&2
	exit 1
}

[ "$(id -u)" -ne 0 ] || die "Run this as the user Klipper runs as, not as root."
[ -d "$DATA" ] || die "$DATA not found: install Klipper and Moonraker first (e.g. with KIAUH)."
command -v systemctl >/dev/null || die "systemd is required."

# Removes our section from moonraker.conf (up to the next section)
remove_update_section() {
	[ -f "$MOONRAKER_CONF" ] || return 0
	python3 - "$MOONRAKER_CONF" "$SECTION" <<'EOF'
import sys
path, section = sys.argv[1], sys.argv[2]
lines = open(path).read().split("\n")
out, skip = [], False
for line in lines:
    if line.strip().startswith("["):
        skip = line.strip() == section
    if not skip:
        out.append(line)
text = "\n".join(out).rstrip("\n") + "\n"
open(path, "w").write(text)
EOF
}

uninstall() {
	say "Removing Klipper Helm"
	sudo systemctl disable --now klipper-helm.service 2>/dev/null || true
	sudo rm -f "$SERVICE" "$POLKIT_RULE"
	sudo systemctl daemon-reload
	remove_update_section
	[ -f "$ASVC" ] && sed -i '/^klipper-helm$/d' "$ASVC"
	rm -rf "$DIR"
	if [ -e "$STATE_DIR/klipperscreen-was-enabled" ]; then
		sudo systemctl enable --now KlipperScreen.service || true
		say "KlipperScreen enabled again"
	fi
	sudo rm -rf "$STATE_DIR"
	sudo systemctl restart moonraker.service || true
	say "Done. The panel settings stay in $CONFIG_DIR/klipper-helm.toml."
}

if [ "${1:-}" = "--uninstall" ]; then
	uninstall
	exit 0
fi

case "$(dpkg --print-architecture 2>/dev/null || uname -m)" in
arm64 | aarch64) ASSET=klipper-helm-linux-arm64 ;;
armhf | armv7l) ASSET=klipper-helm-linux-armhf ;;
amd64 | x86_64) ASSET=klipper-helm-linux-amd64 ;;
*) die "Unsupported system architecture: $(uname -m)" ;;
esac

# Moonraker refuses to update an application that lives inside a git repository
if [ -d "$DIR/.git" ]; then
	die "$DIR is a git repository (the source code?). Move it away and run the installer again."
fi

say "Installing the libraries the panel needs"
sudo apt-get update -qq
sudo apt-get install -y -qq libinput10 libxkbcommon0 libudev1 libfontconfig1 fonts-dejavu-core

say "Downloading the latest release"
VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
	python3 -c 'import json, sys; print(json.load(sys.stdin)["tag_name"])')
mkdir -p "$DIR"
curl -fL --progress-bar -o "$DIR/$ASSET.tmp" "https://github.com/$REPO/releases/download/$VERSION/$ASSET"
chmod 755 "$DIR/$ASSET.tmp"
mv "$DIR/$ASSET.tmp" "$DIR/$ASSET"
# Tells Moonraker's update manager what is installed (it rewrites the version on updates)
cat >"$DIR/release_info.json" <<EOF
{
  "project_name": "klipper-helm",
  "project_owner": "${REPO%%/*}",
  "version": "$VERSION",
  "asset_name": "$ASSET"
}
EOF
say "Klipper Helm $VERSION installed in $DIR"

# Settings the panel cannot change itself; editable from Mainsail like printer.cfg
if [ ! -e "$CONFIG_DIR/klipper-helm.env" ]; then
	cat >"$CONFIG_DIR/klipper-helm.env" <<'EOF'
# Klipper Helm: settings applied when the panel starts. Restart it after changes
# (Mainsail: the power menu, or `sudo systemctl restart klipper-helm`).
# Everything else is set on the panel (System -> Panel settings) and saved in klipper-helm.toml.

# Screen mode, when the screen reports a wrong one (black or distorted picture). Usually
# better: video=HDMI-A-1:1024x600@60 in /boot/firmware/cmdline.txt, which also fixes the
# boot screen and is picked up by the panel automatically.
#KLIPPER_HELM_MODE=1024x600

# Screen rotation: 0, 90, 180 or 270
#SLINT_KMS_ROTATION=180

# UI scale; by default computed from the screen size (800x480 = 1)
#SLINT_SCALE_FACTOR=1.25
EOF
fi

say "Installing the klipper-helm service"
sudo tee "$SERVICE" >/dev/null <<EOF
[Unit]
Description=Klipper Helm touch screen panel
Documentation=https://github.com/$REPO
After=moonraker.service
Wants=moonraker.service
# Both draw directly on the screen
Conflicts=KlipperScreen.service

[Service]
Type=simple
User=$USER
SupplementaryGroups=video input render
Environment=KLIPPER_HELM_CONFIG=$CONFIG_DIR/klipper-helm.toml
Environment=KLIPPER_HELM_MANAGE_NETWORK=1
EnvironmentFile=-/etc/default/locale
EnvironmentFile=-$CONFIG_DIR/klipper-helm.env
ExecStart=$DIR/$ASSET
Restart=always
RestartSec=3
# Klipper must never wait for the screen
Nice=10
IOSchedulingClass=idle

[Install]
WantedBy=multi-user.target
EOF

# Network screen: lets this user change NetworkManager's settings without a login session
if command -v nmcli >/dev/null && systemctl is-active --quiet NetworkManager; then
	sudo tee "$POLKIT_RULE" >/dev/null <<EOF
// Klipper Helm (running as $USER) may change NetworkManager settings from the panel
polkit.addRule(function(action, subject) {
    if (action.id.indexOf("org.freedesktop.NetworkManager.") == 0 && subject.user == "$USER") {
        return polkit.Result.YES;
    }
});
EOF
else
	warn "NetworkManager is not running: the panel's Network screen will only show the status."
fi

# Updates from Mainsail (and the panel's Updates screen), restarts through Moonraker
if [ -f "$MOONRAKER_CONF" ] && ! grep -qxF "$SECTION" "$MOONRAKER_CONF"; then
	cat >>"$MOONRAKER_CONF" <<EOF

$SECTION
type: executable
channel: stable
repo: $REPO
path: $DIR
managed_services: klipper-helm
EOF
fi
if [ -f "$ASVC" ] && ! grep -qx klipper-helm "$ASVC"; then
	echo klipper-helm >>"$ASVC"
fi

# Only one program can draw on the screen; --uninstall brings KlipperScreen back
if systemctl is-enabled --quiet KlipperScreen.service 2>/dev/null; then
	sudo mkdir -p "$STATE_DIR"
	sudo touch "$STATE_DIR/klipperscreen-was-enabled"
	sudo systemctl disable --now KlipperScreen.service
	say "KlipperScreen disabled"
fi

sudo systemctl daemon-reload
sudo systemctl enable klipper-helm.service
sudo systemctl restart moonraker.service
sudo systemctl restart klipper-helm.service
say "Done. Klipper Helm is running; updates appear in Mainsail's update manager."
