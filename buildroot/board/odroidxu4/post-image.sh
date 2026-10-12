#!/bin/sh
# Assembles sdcard.img from the boot chain, kernel, root and data partitions.
set -e

BOARD_DIR="$(dirname "$0")"
GENIMAGE_TMP="${BUILD_DIR}/genimage.tmp"

cp "${BOARD_DIR}/extlinux.conf" "${BINARIES_DIR}/extlinux.conf"

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
