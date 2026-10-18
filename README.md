# Klipper Helm

A touch screen panel for Klipper/Moonraker, a KlipperScreen replacement written in Rust + Slint.
On a Raspberry Pi it draws directly through KMS/DRM (no X11/Wayland); on a PC it runs in a window.

## Screens

- **Home**: built to be read from across the room. The remaining time and end time dominate, next to the
  print thumbnail and progress; nozzle, bed and fan chips open their controls. **Tune** adjusts speed, flow,
  Z offset and the part fan during a print. When idle or finished it shows the state and print time.
- **Menu** (a button on Home, and in the header elsewhere) opens every other screen as large tiles:
- **Temperature**: all heaters and sensors, setting targets with a numeric keypad, presets
- **Move**: jogging with a selectable step, homing, motors off
- **Filament**: extrude and retract, fan control
- **Files**: G-code list with thumbnails, metadata and print start
- **Macros**: every `gcode_macro` (except those prefixed with `_`), with confirmation
- **Console**: G-code history and an on-screen keyboard (a physical keyboard works too)
- **System**: Klipper/firmware restart, host reboot/shutdown, panel settings, **Network**
  (Ethernet DHCP or static address, Wi-Fi scanning and joining, through NetworkManager) and
  **Updates** (Moonraker's update manager: Klipper, Moonraker, Mainsail, the panel itself and the
  system packages, as in Mainsail)
- **Move → Calibration**: Z offset (probe/BLTouch paper test, or the Z endstop) and the bed mesh
  (height map, calibration, profiles)
- A **screen saver** with a dimmed clock and the print progress after a few minutes without touches

The emergency stop is always in the top right corner: a red round button with three clockwise arrows,
like the "twist to release" marking on a machine's emergency stop. It acts immediately, like Klipper's own.

## Running on a PC (no printer needed)

```sh
cargo run --example mock_moonraker          # fake Moonraker on 127.0.0.1:7125
cargo run                                   # panel in a window
```

With a real printer: `cargo run` and set the address in System → Panel settings
(or `MOONRAKER_URL=ws://<printer-ip>:7125/websocket cargo run` if the config file has no address).

In VS Code, the tasks in `.vscode/` start the mock and the panel together (Ctrl+Shift+P → "Tasks: Run Task").

Snapshots of all views (debug builds only): `KLIPPER_HELM_SNAPSHOT=<dir> cargo run` saves `page-N.ppm` files and quits.

## Installing on the printer's computer

The panel runs on the computer that runs Klipper (e.g. a Raspberry Pi 4 with Raspberry Pi OS or
MainsailOS, Debian bookworm or newer), next to Moonraker and Mainsail. Connect the screen to it (HDMI or DSI,
touch over USB) and run, as the user Klipper runs as:

```sh
curl -fsSL https://raw.githubusercontent.com/wsalata/klipper-helm/main/install.sh | bash
```

The installer:

- downloads the latest release for the system (`klipper-helm-linux-arm64`, `-armhf` or `-amd64`) into
  `~/klipper-helm`, with the few libraries it needs;
- installs the `klipper-helm` systemd service (lower priority than Klipper) and disables KlipperScreen,
  since only one program can draw on the screen;
- adds `[update_manager klipper-helm]` (type `executable`) to `moonraker.conf` and the service to
  `moonraker.asvc`, so updates appear in Mainsail next to Klipper's and Moonraker's, and Mainsail can
  restart the panel;
- with NetworkManager (Raspberry Pi OS / MainsailOS on bookworm or newer), adds a polkit rule that
  lets the panel change the network settings from the Network screen.

`bash install.sh --uninstall` removes it all again and re-enables KlipperScreen if it was in use.

On a Raspberry Pi, `--boot-splash` also replaces the boot screen (the rainbow square, the raspberries and
the boot messages) with the Klipper Helm logo, shown a few seconds after power-on and kept until the
panel draws its first frame, at the same spot as the panel's welcome screen:

```sh
curl -fsSL https://raw.githubusercontent.com/wsalata/klipper-helm/main/install.sh | bash -s -- --boot-splash
```

It installs Plymouth with the theme from [packaging/plymouth](packaging/plymouth) (rebuilding the
initramfs, together with corrected screen EDIDs from `/lib/firmware/edid` if there are any), adds `disable_splash=1` to `config.txt`, and in `cmdline.txt` moves the console to `tty3` and
adds `quiet splash loglevel=3 logo.nologo vt.global_cursor_default=0 plymouth.ignore-serial-consoles`.
Both files are backed up first (`*.klipper-helm.bak`); `--uninstall` undoes exactly these changes. The
logo is rendered from the panel itself by `packaging/plymouth/make-logo.sh`; run it again if the welcome
screen changes.

The panel draws through DRM/KMS without X11 or Wayland: HDMI and DSI screens work, as do SPI screens with
a DRM driver (e.g. `panel-mipi-dbi`); SPI screens with only an `fbtft` framebuffer driver do not.
Screen rotation and the UI scale go in `~/printer_data/config/klipper-helm.env`, next to the panel
settings (`klipper-helm.toml`); both can be edited in Mainsail.

### Releasing

Set the version in `Cargo.toml`, commit and push a matching tag (`git tag v0.3.0 && git push --tags`).
The release workflow builds the binaries (cross-compiled in Debian bookworm, for glibc 2.36 and newer) and publishes them;
Moonraker then offers the update. **Actions → Build → Run workflow** builds them without releasing.

### Building on the Pi

```sh
sudo apt install libudev-dev libinput-dev libxkbcommon-dev libfontconfig-dev
cargo build --release --no-default-features --features embedded
```

## Troubleshooting

Where to look on the printer's computer:

- `journalctl -u klipper-helm -n 50` – the panel's log. At start it names the screen and mode it uses
  (`Display HDMI-A-1: 1024x600@59`).
- `sudo dmesg | grep -i drm` – the kernel's view of the screen.
- `grep -l '^connected' /sys/class/drm/*/status` – the port the screen is on (e.g. `card1-HDMI-A-1`);
  `cat /sys/class/drm/card1-HDMI-A-1/modes` lists the modes the kernel can use for it.

### Black screen or wrong resolution with a cheap HDMI screen

Seen with a Waveshare 7inch HDMI LCD (C) (1024x600) on a Raspberry Pi 4 with MainsailOS.

**Symptoms:** the boot messages have a wrong resolution; the panel stays black until the screen is
unplugged and plugged in again, or does not start at all. The panel's log says
`Could not initialize software display`, `dmesg` repeats
`User-defined mode not supported: "1024x600": 60 49000 1024 1029 1042 1312 …` and
`Cannot find any crtc or sizes`, the `modes` file is empty and there is no `/dev/fb0`.

**Cause:** the screen describes itself (its EDID) with a single 1024x600 mode whose horizontal timing
differs from the manufacturer's specification and has odd values (front porch 5, sync 13). The Raspberry
Pi 4's HDMI output can only produce even horizontal timings, so the kernel drops the screen's only mode
and has nothing to show. The screen also does not recover after the signal disappears or changes, which
is why replugging helps. On a PC the same screen usually works.

**Fix:** give the kernel a corrected copy of the EDID with the manufacturer's timing (for this screen:
49 MHz, horizontal 1024 + 48/96/144, vertical 600 + 3/10/11), and set the same mode for the firmware's
boot screen. On the printer's computer:

1. Save the screen's EDID and write the corrected copy (adjust `card1-HDMI-A-1` to the port found above):

   ```sh
   cp /sys/class/drm/card1-HDMI-A-1/edid ~/edid.bin
   python3 - ~/edid.bin ~/screen-1024x600.bin <<'EOF'
   import sys
   edid = bytearray(open(sys.argv[1], "rb").read()[:128])
   # First detailed timing: 49 MHz, 1024 + 48/96/144, 600 + 3/10/11, 154 x 86 mm
   edid[54:71] = bytes([0x24, 0x13, 0x00, 0x20, 0x41, 0x58, 0x18, 0x20, 0x30,
                        0x60, 0x3A, 0x00, 0x9A, 0x56, 0x00, 0x00, 0x00])
   edid[127] = -sum(edid[:127]) % 256
   open(sys.argv[2], "wb").write(edid)
   EOF
   ```

   `edid-decode ~/screen-1024x600.bin` (package `edid-decode`) should show
   `Hfront 48 Hsync 96 Hback 144` and `Vfront 3 Vsync 10 Vback 11`.

2. Let the kernel load it instead of the screen's own EDID:

   ```sh
   sudo mkdir -p /lib/firmware/edid
   sudo cp ~/screen-1024x600.bin /lib/firmware/edid/
   sudo cp /boot/firmware/cmdline.txt /boot/firmware/cmdline.txt.bak
   ```

   Then edit `/boot/firmware/cmdline.txt`: it is a single line; remove any `video=…` and append
   ` drm.edid_firmware=HDMI-A-1:edid/screen-1024x600.bin` to it. If the system no longer boots, restore
   `cmdline.txt.bak` from the SD card's `bootfs` partition on another computer.

   If the graphics driver starts from the initramfs (as with Plymouth, so with `--boot-splash`), the file
   must be in the initramfs too, or `dmesg` shows `Direct firmware load for edid/… failed with error -2`
   and the screen stays black. `install.sh --boot-splash` takes care of it when the file exists
   (`/etc/initramfs-tools/hooks/klipper-helm-edid`); after installing the splash first, run
   `bash install.sh --boot-splash` again, or `sudo update-initramfs -u -k all` once the hook is there.

3. Set the same resolution for the boot screen, at the end of `/boot/firmware/config.txt` (the
   manufacturer's setting; giving the firmware the EDID file with `hdmi_edid_file` instead showed a
   coloured strip on this screen):

   ```ini
   [all]
   hdmi_group=2
   hdmi_mode=87
   hdmi_cvt=1024 600 60 6 0 0 0
   ```

4. `sudo reboot`. Now `modes` lists `1024x600`, `/dev/fb0` exists, the boot messages are sharp and the
   panel starts without replugging.

Another screen with the same problem needs its own manufacturer's timing in step 1. A screen that only
reports a wrong preferred mode, but one the Pi can output, needs just `video=HDMI-A-1:<width>x<height>@60`
in `cmdline.txt` for the boot screen and `SLINT_DRM_MODE=<index>` in `klipper-helm.env` for the panel
(`SLINT_DRM_MODE=list` prints the indexes).

### No picture after enabling the boot splash

The boot messages are on `tty3` now, so a problem during boot is not visible on the screen; use SSH, or
`sudo journalctl -b`. If the system does not boot at all, put the SD card into another computer and on
its `bootfs` partition replace `cmdline.txt` and `config.txt` with the `*.klipper-helm.bak` copies.

### The panel does not start: no connected display

`No connected display connector found` in the panel's log means the kernel sees no screen: check the
cable and the port (on a Raspberry Pi 4 the micro-HDMI port next to the power socket is `HDMI-A-1`), and
that `dtoverlay=vc4-kms-v3d` is in `config.txt`. If the screen is detected but has no modes, see above.

## Thumbnails

The panel shows the thumbnails a slicer embeds in the G-code file, which Moonraker extracts into `.thumbs`.
Enable them in PNG format in the slicer, e.g. in PrusaSlicer/OrcaSlicer: Printer Settings → G-code thumbnails
`48x48/PNG, 300x300/PNG`. The small one is used in the file list, the large one in the file dialog and on
the Home screen during a print.

## Configuration

Settings (Moonraker address, language, manual movement speeds, temperature presets, screen saver) are changed on the
**System → Panel settings** screen. Saving applies them immediately: changing the address switches the panel
to another printer without a restart. The screen also works without a connection, so a wrong address can be fixed.

Settings are stored in `~/.config/klipper-helm/config.toml`, or the file given by `KLIPPER_HELM_CONFIG`
(`~/printer_data/config/klipper-helm.toml` when installed with `install.sh`), see
[config.example.toml](config.example.toml). The address in the file takes precedence over `MOONRAKER_URL`.

## Look and fonts

The UI follows one style ("at a glance"): a deep navy ground, a teal accent and the Saira typeface.
The fonts are compiled into the binary from `ui/fonts/` (SIL Open Font License, `ui/fonts/OFL.txt`),
so the Pi needs no extra font packages; colors and sizes live in `ui/theme.slint`.

## Translations

UI text is written in English in the `.slint` files and marked with `@tr("...")`. Translations are compiled
into the binary from `ui/translations/<lang>/LC_MESSAGES/klipper-helm.po` (gettext format, no message context).
The language follows the system locale (`LANG`) or the choice in Panel settings, and can be switched live.

Rust code never produces user-visible text: it sends codes (e.g. `Status`, heater `role`) and Slint turns them
into translated labels (`Strings` and `Messages` in `ui/state.slint`).

To add or change a string: use `@tr("...")` in Slint, then add the `msgid`/`msgstr` pair to the `.po` file.
The template can be regenerated with
[`slint-tr-extractor`](https://docs.slint.dev/latest/docs/slint/guide/development/translations/):

```sh
cargo install slint-tr-extractor
find ui -name '*.slint' | xargs slint-tr-extractor --no-default-translation-context -o klipper-helm.pot
msgmerge -U ui/translations/pl/LC_MESSAGES/klipper-helm.po klipper-helm.pot
```

A new language is a new directory, e.g. `ui/translations/de/LC_MESSAGES/klipper-helm.po`, plus an entry in
`LANGUAGES` (`src/config.rs`) and a button in `ui/pages/settings.slint`.
