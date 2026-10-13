#!/bin/sh
# Assembles sdcard.img, and the signed update bundle when the signing key is available.
set -e

BOARD_DIR="$(dirname "$0")"
KEYS_DIR="${BOARD_DIR}/../../keys"
GENIMAGE_TMP="${BUILD_DIR}/genimage.tmp"
VERSION="$(cat "${TARGET_DIR}/etc/klipper-helm-release")"

# A/B boot script; compiled here so every image has the current boot.cmd
"${HOST_DIR}/bin/mkimage" -C none -A arm -T script -d "${BOARD_DIR}/boot.cmd" "${BINARIES_DIR}/boot.scr"

# Hardkernel's bl1 is 256 bytes longer than its slot; their sd_fusing.sh simply
# lets bl2 overwrite the tail, so trim it to fit (hardkernel/u-boot issue #45).
truncate -s 15360 "${BINARIES_DIR}/bl1.bin.hardkernel"

rm -rf "${GENIMAGE_TMP}"
genimage \
	--rootpath "${TARGET_DIR}" \
	--tmppath "${GENIMAGE_TMP}" \
	--inputpath "${BINARIES_DIR}" \
	--outputpath "${BINARIES_DIR}" \
	--config "${BOARD_DIR}/genimage.cfg"

# RAUC update bundle: the new root filesystem, installed into the inactive slot
KEY="${KLIPPER_HELM_SIGNING_KEY:-${KEYS_DIR}/release.key.pem}"
CERT="${KEYS_DIR}/release.cert.pem"
BUNDLE="${BINARIES_DIR}/klipper-helm-odroidxu4-${VERSION}.raucb"
if [ ! -f "${KEY}" ]; then
	echo "No signing key (${KEY}); skipping the update bundle" >&2
	exit 0
fi
BUNDLE_DIR="${BUILD_DIR}/rauc-bundle"
rm -rf "${BUNDLE_DIR}" "${BINARIES_DIR}"/klipper-helm-odroidxu4-*.raucb
mkdir -p "${BUNDLE_DIR}"
cp -L "${BINARIES_DIR}/rootfs.ext4" "${BUNDLE_DIR}/rootfs.ext4"
cat > "${BUNDLE_DIR}/manifest.raucm" <<MANIFEST
[update]
compatible=klipper-helm-odroidxu4
version=${VERSION}

[bundle]
format=verity

[image.rootfs]
filename=rootfs.ext4
MANIFEST
# xz: a smaller download (the kernel has SQUASHFS_XZ)
"${HOST_DIR}/bin/rauc" bundle --cert="${CERT}" --key="${KEY}" \
	--mksquashfs-args="-comp xz -b 1M" "${BUNDLE_DIR}" "${BUNDLE}"
echo "Update bundle: ${BUNDLE}"
