#!/bin/sh
# Runs after the target filesystem is assembled, before the images are made.
set -e
TARGET_DIR="$1"

# Mount point for the writable data partition (see /etc/init.d/S15data)
mkdir -p "${TARGET_DIR}/data"

# Start the panel once the rc scripts have run, and restart it if it exits
grep -q klipper-helm-start "${TARGET_DIR}/etc/inittab" || \
	sed -i '/^# Put a getty on the serial port/i # Klipper Helm on the display\n::respawn:/usr/bin/klipper-helm-start\n' \
		"${TARGET_DIR}/etc/inittab"
