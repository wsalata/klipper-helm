# Klipper Helm

**English** | [Polski](README.pl.md)

A touch screen panel for Klipper/Moonraker, a KlipperScreen replacement written in Rust + Slint.
On a Raspberry Pi it draws directly through KMS/DRM (no X11/Wayland); on a PC it runs in a window.

## Screens

- **Home**: heaters, print progress with thumbnail, elapsed/remaining time, layer, pause/resume/cancel,
  speed, flow and Z offset adjustment during a print
- **Temp.**: all heaters and sensors, setting targets with a numeric keypad, presets
- **Move**: jogging with a selectable step, homing, motors off
- **Filament**: extrude and retract, fan control
- **Files**: G-code list with thumbnails, metadata and print start
- **Macros**: every `gcode_macro` (except those prefixed with `_`), with confirmation
- **Console**: G-code history and an on-screen keyboard (a physical keyboard works too)
- **System**: Klipper/firmware restart, host reboot/shutdown, panel settings

The **STOP** button (emergency stop) is always visible in the top bar.

## Running on a PC (no printer needed)

```sh
cargo run --example mock_moonraker          # fake Moonraker on 127.0.0.1:7125
cargo run                                   # panel in a window
```

With a real printer: `cargo run` and set the address in System → Panel settings
(or `MOONRAKER_URL=ws://<printer-ip>:7125/websocket cargo run` if the config file has no address).

In VS Code, the tasks in `.vscode/` start the mock and the panel together (Ctrl+Shift+P → "Tasks: Run Task").

Snapshots of all views (debug builds only): `KLIPPER_HELM_SNAPSHOT=<dir> cargo run` saves `page-N.ppm` files and quits.

## Installing on a Raspberry Pi

```sh
sudo apt install libudev-dev libinput-dev libxkbcommon-dev libgbm-dev libdrm-dev libfontconfig-dev
cargo build --release
sudo cp klipper-helm.service /etc/systemd/system/
sudo systemctl disable --now KlipperScreen   # if it was installed
sudo systemctl enable --now klipper-helm
```

Add `klipper-helm` to `~/printer_data/moonraker.asvc` so Moonraker can restart the service.

Scaling the UI on screens other than 800×480: `Environment=SLINT_SCALE_FACTOR=1.5` in the service file.
Rotating the screen: `Environment=SLINT_KMS_ROTATION=180`.

## Thumbnails

The panel shows the thumbnails a slicer embeds in the G-code file, which Moonraker extracts into `.thumbs`.
Enable them in PNG format in the slicer, e.g. in PrusaSlicer/OrcaSlicer: Printer Settings → G-code thumbnails
`48x48/PNG, 300x300/PNG`. The small one is used in the file list, the large one in the file dialog and on
the Home screen during a print.

## Configuration

Settings (Moonraker address, language, manual movement speeds, temperature presets) are changed on the
**System → Panel settings** screen. Saving applies them immediately: changing the address switches the panel
to another printer without a restart. The screen also works without a connection, so a wrong address can be fixed.

Settings are stored in `~/.config/klipper-helm/config.toml` (or the file given by `KLIPPER_HELM_CONFIG`),
see [config.example.toml](config.example.toml). The address in the file takes precedence over `MOONRAKER_URL`.

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
