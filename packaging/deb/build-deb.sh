#!/bin/sh
# Packages an already built panel binary as a Debian package for Raspberry Pi OS.
#   packaging/deb/build-deb.sh <binary> <arch: arm64|armhf|amd64> <version> [output dir]
# Built on Debian bookworm, the package needs glibc 2.36 (Raspberry Pi OS bookworm or newer).
set -eu

BINARY=$1
ARCH=$2
VERSION=$3
OUT=${4:-.}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
PKG="$STAGE/klipper-helm"

install -D -m 0755 "$BINARY" "$PKG/usr/bin/klipper-helm"
install -D -m 0755 "$HERE/klipper-helm-start" "$PKG/usr/lib/klipper-helm/klipper-helm-start"
install -D -m 0644 "$HERE/klipper-helm.service" "$PKG/usr/lib/systemd/system/klipper-helm.service"
install -D -m 0644 "$HERE/default" "$PKG/etc/default/klipper-helm"
install -D -m 0644 "$ROOT/ui/fonts/OFL.txt" "$PKG/usr/share/doc/klipper-helm/fonts-OFL.txt"
install -D -m 0644 "$ROOT/config.example.toml" "$PKG/usr/share/doc/klipper-helm/config.example.toml"

mkdir -p "$PKG/DEBIAN"
for script in postinst prerm postrm; do
	install -m 0755 "$HERE/$script" "$PKG/DEBIAN/$script"
done
echo /etc/default/klipper-helm > "$PKG/DEBIAN/conffiles"

size=$(du -sk "$PKG" --exclude=DEBIAN | cut -f1)
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: klipper-helm
Version: $VERSION
Architecture: $ARCH
Maintainer: Walery Sałata <walery.salata@gmail.com>
Installed-Size: $size
Depends: libc6 (>= 2.36), libgcc-s1, libinput10, libxkbcommon0, libudev1, libfontconfig1, fonts-dejavu-core, systemd
Section: misc
Priority: optional
Homepage: https://github.com/wsalata/klipper-helm
Description: Touch screen panel for Klipper (KlipperScreen replacement)
 Klipper Helm controls a 3D printer running Klipper through Moonraker from a
 touch screen. It draws directly on the display (DRM/KMS), without X11 or Wayland,
 and runs as the klipper-helm systemd service.
CONTROL

mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$PKG" "$OUT/klipper-helm_${VERSION}_${ARCH}.deb"
