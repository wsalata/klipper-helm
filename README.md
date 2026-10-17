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

The panel draws through DRM/KMS without X11 or Wayland: HDMI and DSI screens work, as do SPI screens with
a DRM driver (e.g. `panel-mipi-dbi`); SPI screens with only an `fbtft` framebuffer driver do not.
Screen rotation and the UI scale go in `~/printer_data/config/klipper-helm.env`, next to the panel
settings (`klipper-helm.toml`); both can be edited in Mainsail.

Cheap HDMI screens often report a wrong preferred mode (e.g. 1024x768 for a 1024x600 panel), which
shows as a wrong resolution while the system boots and a black or distorted panel. Add
`video=HDMI-A-1:1024x600@60` (with the screen's real size and the connector from
`ls /sys/class/drm`) to the single line in `/boot/firmware/cmdline.txt`: it fixes the boot screen,
and the panel uses that mode too. Alternatively set `KLIPPER_HELM_MODE=1024x600` in
`klipper-helm.env`; the panel's log (`journalctl -u klipper-helm`) shows the mode in use.

### Releasing

Set the version in `Cargo.toml`, commit and push a matching tag (`git tag v0.3.0 && git push --tags`).
The release workflow builds the binaries (cross-compiled in Debian bookworm, for glibc 2.36 and newer) and publishes them;
Moonraker then offers the update. **Actions → Build → Run workflow** builds them without releasing.

### Building on the Pi

```sh
sudo apt install libudev-dev libinput-dev libxkbcommon-dev libfontconfig-dev
cargo build --release --no-default-features --features embedded
```

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
