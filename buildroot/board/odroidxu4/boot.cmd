# Klipper Helm A/B boot (RAUC). Boots the first slot in BOOT_ORDER that has tries left
# and counts the try down; the booted system marks itself good (S99rauc-mark-good),
# which restores its counter. A system that keeps failing to boot runs out of tries,
# so U-Boot falls back to the other slot. Partitions: 2 = slot A, 3 = slot B.

test -n "${BOOT_ORDER}" || setenv BOOT_ORDER "A B"
test -n "${BOOT_A_LEFT}" || setenv BOOT_A_LEFT 3
test -n "${BOOT_B_LEFT}" || setenv BOOT_B_LEFT 3

setenv bootpart
setenv raucslot
for BOOT_SLOT in "${BOOT_ORDER}"; do
	if test -z "${bootpart}"; then
		if test "x${BOOT_SLOT}" = "xA"; then
			if test ${BOOT_A_LEFT} -gt 0; then
				setexpr BOOT_A_LEFT ${BOOT_A_LEFT} - 1
				setenv bootpart 2
				setenv raucslot A
			fi
		fi
		if test "x${BOOT_SLOT}" = "xB"; then
			if test ${BOOT_B_LEFT} -gt 0; then
				setexpr BOOT_B_LEFT ${BOOT_B_LEFT} - 1
				setenv bootpart 3
				setenv raucslot B
			fi
		fi
	fi
done

if test -z "${bootpart}"; then
	echo "No slot has boot tries left; resetting the counters"
	setenv BOOT_A_LEFT 3
	setenv BOOT_B_LEFT 3
	saveenv
	reset
fi
saveenv

# loglevel=5 instead of quiet: quiet would also hide the boot logo
setenv bootargs "console=ttySAC2,115200n8 root=PARTUUID=4b484c4d-0${bootpart} rauc.slot=${raucslot} rootwait ro loglevel=5 fbcon=logo-pos:center net.ifnames=0 vt.global_cursor_default=0"
echo "Booting slot ${raucslot} (partition ${bootpart})"
load ${devtype} ${devnum}:${bootpart} ${kernel_addr_r} /boot/zImage
load ${devtype} ${devnum}:${bootpart} ${fdt_addr_r} /boot/exynos5422-odroidxu4.dtb
bootz ${kernel_addr_r} - ${fdt_addr_r}
