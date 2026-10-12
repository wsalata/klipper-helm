# Embedded image (Buildroot)

A small, read-only Linux image that boots straight into Klipper Helm. It contains only the panel:
Klipper and Moonraker run on another machine, and the panel connects to them over the network.

Target: **Odroid XU4** (Exynos 5422) with an HDMI screen and a USB touch screen.

## What is in the image

- Mainline Linux 6.12 LTS (`exynos_defconfig` + [linux.fragment](board/odroidxu4/linux.fragment)):
  HDMI through exynos-drm, USB HID touch screens, the RTL8153 Gigabit Ethernet port, the fan.
- Mainline U-Boot (`odroid-xu3`, which covers the XU4) behind Hardkernel's signed BL1/BL2/TZSW blobs
  (package [odroidxu4-blobs](package/odroidxu4-blobs)), booting through
  [extlinux.conf](board/odroidxu4/extlinux.conf).
- BusyBox init, eudev, DHCP on `eth0`, `ntpd` for the clock, time zone Europe/Warsaw.
- The panel built with `--no-default-features --features embedded`: KMS output with Slint's software
  renderer, so no GPU driver is needed (Mali-T628 support in Mesa is not reliable).

## Boot and runtime

- Kernel messages go to the serial console (`ttySAC2`, 115200 baud) only; the screen stays dark until
  the panel appears.
- `/etc/inittab` starts [klipper-helm-start](board/odroidxu4/rootfs-overlay/usr/bin/klipper-helm-start)
  and restarts it whenever the panel exits. Its output goes to `/tmp/klipper-helm.log`.
- The root filesystem is mounted read-only, so pulling the plug cannot corrupt it. The settings live on
  a separate `data` partition mounted on `/data` and checked with `fsck` at boot
  ([S15data](board/odroidxu4/rootfs-overlay/etc/init.d/S15data)).
- Set the Moonraker address in **System → Panel settings**; it is saved to
  `/data/klipper-helm/config.toml` and survives reboots.
- Per-device overrides go in `/data/klipper-helm/env`, e.g. `SLINT_KMS_ROTATION=180` or
  `SLINT_SCALE_FACTOR=1.25`.

## Building

On a Linux PC with the usual Buildroot host tools (gcc, g++, make, perl, rsync, cpio, unzip, bc, file, wget)
and network access:

```sh
make image
```

The first run downloads Buildroot 2026.08 into `.buildroot/` (checked against its published SHA-256),
builds the cross toolchain, kernel, U-Boot and the panel, and writes
`.buildroot/output/images/sdcard.img`. It takes a while the first time; later runs reuse everything.

- `make panel-rebuild` rebuilds only the panel after code changes, then the image.
- `make menuconfig` changes the Buildroot configuration (saved in `.buildroot/output/.config`; copy
  changes you want to keep into [the defconfig](configs/klipper_helm_odroidxu4_defconfig)).

The panel package builds from this repository, including uncommitted changes, and vendors its crates
with `cargo vendor` during the build, so the build needs network access.

## Flashing

Write the image to a microSD card or the eMMC module (replace `/dev/sdX`, check it first with `lsblk`):

```sh
sudo dd if=.buildroot/output/images/sdcard.img of=/dev/sdX bs=4M conv=fsync status=progress
```

or use balenaEtcher. On the XU4, set the boot selector switch to the medium you flashed.

## Troubleshooting

- Nothing on the screen: connect a USB-serial adapter to the UART header (1.8 V levels) to see the boot
  log, then log in as `root` (no password) and check `/tmp/klipper-helm.log`.
- Touch does not respond: `cat /proc/bus/input/devices` should list the touch controller.
