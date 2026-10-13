# Embedded image (Buildroot)

A small, read-only Linux image that boots straight into Klipper Helm. It contains only the panel:
Klipper and Moonraker run on another machine, and the panel connects to them over the network.

Target: **Odroid XU4** (Exynos 5422) with an HDMI screen and a USB touch screen.

## What is in the image

- Mainline Linux 6.12 LTS (`exynos_defconfig` + [linux.fragment](board/odroidxu4/linux.fragment)):
  HDMI through exynos-drm, USB HID touch screens, the RTL8153 Gigabit Ethernet port, the fan.
  [A patch](board/odroidxu4/patches/linux) adds Hardkernel's HDMI PHY settings and picks the nearest
  one within 2%: mainline knows too few, so small screens (800x480) otherwise stay dark.
- Mainline U-Boot (`odroid-xu3`, which covers the XU4) behind Hardkernel's signed BL1/BL2/TZSW blobs
  (package [odroidxu4-blobs](package/odroidxu4-blobs)), booting through
  [extlinux.conf](board/odroidxu4/extlinux.conf).
- BusyBox init, eudev, `ntpd` for the clock, time zone Europe/Warsaw.
- Networking: `dhcpcd` on every interface, `wpa_supplicant` for USB Wi-Fi adapters (drivers as modules
  for Realtek RTL8188/8192/8723/8821/8822, MediaTek MT7601U/MT7921AU, Ralink RT2870/RT3070/RT5370 and
  Atheros AR9271, with their firmware). Interfaces are named `eth0` and `wlan0` (`net.ifnames=0`).
- The panel built with `--no-default-features --features embedded`: KMS output with Slint's software
  renderer, so no GPU driver is needed (Mali-T628 support in Mesa is not reliable).

## Boot and runtime

- Kernel messages go to the serial console (`ttySAC2`, 115200 baud) only. The screen shows the
  Klipper Helm logo from the moment HDMI starts (kernel boot logo, centred) until the panel takes over
  with the same logo while it connects. The logo image is
  [boot-logo.png](board/odroidxu4/boot-logo.png), rendered from `ui/icons/logo.svg` and the panel's
  fonts by [make-boot-logo.sh](board/odroidxu4/make-boot-logo.sh); `quiet` is not used because it
  also hides the logo (`loglevel=5` keeps the serial console short instead).
- `/etc/inittab` starts [klipper-helm-start](board/odroidxu4/rootfs-overlay/usr/bin/klipper-helm-start)
  and restarts it whenever the panel exits. Its output goes to `/data/logs/klipper-helm.log`
  (see Troubleshooting).
- The root filesystem is mounted read-only, so pulling the plug cannot corrupt it. The settings live on
  a separate `data` partition mounted on `/data` and checked with `fsck` at boot
  ([S15data](board/odroidxu4/rootfs-overlay/etc/init.d/S15data)).
- Set the Moonraker address in **System → Panel settings**; it is saved to
  `/data/klipper-helm/config.toml` and survives reboots.
- **System → Network** shows the Ethernet and Wi-Fi addresses, switches Ethernet between DHCP and a
  static address, and scans for and joins Wi-Fi networks. Settings are kept in `/data/network/`
  (`dhcpcd.conf`, `wpa_supplicant.conf`), created with defaults (DHCP) on first boot. A USB Wi-Fi
  adapter can be plugged in at any time.
- The UI is designed for 800x480 and is scaled to fill larger screens (1024x600 → 1.25, 1280x720 → 1.5),
  computed at start from the display's preferred mode.
- Per-device overrides go in `/data/klipper-helm/env`, e.g. `SLINT_KMS_ROTATION=180`, or
  `SLINT_SCALE_FACTOR=1` to turn the scaling off.

## Updates

The card holds two root filesystems (A and B), each with its own kernel, plus the `boot` and `data`
partitions. **System → Updates** in the panel checks the GitHub releases of `update_repo` (see
[config.example.toml](../config.example.toml)), downloads the signed update bundle (`.raucb`) to `/data`
and installs it with [RAUC](https://rauc.io) into the slot that is not running. After a restart U-Boot
([boot.cmd](board/odroidxu4/boot.cmd)) boots the new slot; if it fails to boot three times, U-Boot goes
back to the previous one. A system that boots completely marks itself good
([S99rauc-mark-good](board/odroidxu4/rootfs-overlay/etc/init.d/S99rauc-mark-good)). Settings on `/data`
are kept.

- Bundles must be signed with the key matching `buildroot/keys/release.cert.pem`, which is built into the
  image; anything else is rejected.
- A private repository needs `update_token` in the config file (a GitHub token with read-only access to
  the repository's contents).
- Images built before the A/B layout (a single root partition) cannot be updated this way: flash the
  new image once.

## Releasing a version

One-time setup:

1. `buildroot/keys/create-keys.sh` creates `release.cert.pem` (commit it) and `release.key.pem`
   (never commit it; keep a backup: devices only accept updates signed with this key).
2. In the GitHub repository, add the secret `RAUC_SIGNING_KEY` with the content of `release.key.pem`.

For each release:

1. Set the new version in `Cargo.toml` and commit.
2. Tag and push: `git tag v0.2.0 && git push github v0.2.0`.
3. [The release workflow](../.github/workflows/release.yml) builds the image and the update bundle and
   publishes them as a GitHub release (`klipper-helm-odroidxu4-<version>.img.xz` for flashing,
   `.raucb` for updates). Panels then offer the update.

Local builds sign the bundle with `buildroot/keys/release.key.pem` if it exists and get the version
`<Cargo.toml version>-dev`, which every release counts as newer.

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

On every boot the panel's start script saves diagnostics to the `data` partition, in `logs/`:
`dmesg.txt` (kernel log with display mode decisions), `drm.txt` (connectors and the modes they accept),
`edid-*.bin` (what the screen reports), `input.txt` (touch devices) and `klipper-helm.log` (the panel).
Read them on a Linux PC:

```sh
sudo mount -L data /mnt && ls /mnt/logs && sudo umount /mnt
```

- Small HDMI screens are often powered by their USB touch cable: without it the screen stays dark
  and `drm.txt` lists only the fallback mode 640x480 (no EDID). A charge-only cable powers the screen
  but the touch controller does not appear in `input.txt`.
- Screen dark although powered: add `drm.debug=0x4 console=tty1` to the `append` line in
  `extlinux/extlinux.conf` on the boot partition. Boot messages then appear on the screen, and
  `dmesg.txt` shows the screen's modes and `Could not find phy config` for any that were rejected.
- `drm.txt` shows the mode but the panel does not appear: see `klipper-helm.log`.
- A USB-serial adapter on the UART header (1.8 V levels) shows the boot log live; log in as `root`
  (no password).
- Touch does not respond: `cat /proc/bus/input/devices` should list the touch controller.
