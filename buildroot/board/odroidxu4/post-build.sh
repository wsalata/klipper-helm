#!/bin/sh
# Runs after the target filesystem is assembled, before the images are made.
set -e
TARGET_DIR="$1"

# Mount point for the writable data partition (see /etc/init.d/S15data)
mkdir -p "${TARGET_DIR}/data"

# Network settings are edited from the panel and kept on /data (see S37network-config);
# dhcpcd keeps its leases in RAM, as the root filesystem is read-only
ln -sfn /data/network/dhcpcd.conf "${TARGET_DIR}/etc/dhcpcd.conf"
rm -rf "${TARGET_DIR}/var/lib/dhcpcd"
ln -sfn /tmp/dhcpcd "${TARGET_DIR}/var/lib/dhcpcd"

# dhcpcd handles all addresses; ifup (S40network) only brings up loopback
cat > "${TARGET_DIR}/etc/network/interfaces" <<'IFACES'
# Addresses are managed by dhcpcd (System -> Network in the panel)
auto lo
iface lo inet loopback
IFACES

# Start the panel once the rc scripts have run, and restart it if it exits
grep -q klipper-helm-start "${TARGET_DIR}/etc/inittab" || \
	sed -i '/^# Put a getty on the serial port/i # Klipper Helm on the display\n::respawn:/usr/bin/klipper-helm-start\n' \
		"${TARGET_DIR}/etc/inittab"
