# Klipper Helm

Panel dotykowy dla Klippera/Moonrakera, zamiennik KlipperScreen napisany w Rust + Slint.
Na Raspberry Pi rysuje bezpośrednio przez KMS/DRM (bez X11/Wayland), na PC działa w oknie.

## Ekrany

- **Pulpit**: grzałki, postęp wydruku, czas/ETA, warstwa, pauza/wznów/anuluj, regulacja prędkości, przepływu i Z-offsetu w trakcie druku
- **Temp.**: wszystkie grzałki i czujniki, ustawianie temperatury klawiaturą numeryczną, presety
- **Ruch**: przesuwanie osi z wyborem kroku, bazowanie, wyłączanie silników
- **Filament**: ekstruzja i retrakcja, sterowanie wentylatorami
- **Pliki**: lista G-code z metadanymi i startem wydruku
- **Makra**: wszystkie `gcode_macro` (bez prefiksu `_`), z potwierdzeniem
- **Konsola**: historia G-code i klawiatura ekranowa (działa też fizyczna)
- **System**: restart Klippera/firmware, reboot/wyłączenie komputera

Przycisk **STOP** (awaryjne zatrzymanie) jest zawsze widoczny na górnym pasku.

## Uruchomienie na PC (bez drukarki)

```sh
cargo run --example mock_moonraker          # udawany Moonraker na 127.0.0.1:7125
cargo run                                   # panel w oknie
```

Z prawdziwą drukarką: `MOONRAKER_URL=ws://<ip-drukarki>:7125/websocket cargo run`.

Zrzuty wszystkich ekranów (tylko build debug): `KLIPPER_HELM_SNAPSHOT=<katalog> cargo run` zapisuje `page-N.ppm` i kończy pracę.

## Instalacja na Raspberry Pi

```sh
sudo apt install libudev-dev libinput-dev libxkbcommon-dev libgbm-dev libdrm-dev libfontconfig-dev
cargo build --release
sudo cp klipper-helm.service /etc/systemd/system/
sudo systemctl disable --now KlipperScreen   # jeśli był zainstalowany
sudo systemctl enable --now klipper-helm
```

Dopisz `klipper-helm` do `~/printer_data/moonraker.asvc`, żeby Moonraker mógł restartować usługę.

Skalowanie interfejsu na innym ekranie niż 800×480: `Environment=SLINT_SCALE_FACTOR=1.5` w pliku usługi.
Obrót ekranu: `Environment=SLINT_KMS_ROTATION=180`.

## Konfiguracja

Patrz [config.example.toml](config.example.toml).
