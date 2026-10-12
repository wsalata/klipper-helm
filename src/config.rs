use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub moonraker_url: String,
    /// Prędkość ruchu ręcznego XY [mm/s]
    pub xy_speed: f32,
    /// Prędkość ruchu ręcznego Z [mm/s]
    pub z_speed: f32,
    #[serde(rename = "preset")]
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub name: String,
    pub extruder: f32,
    pub bed: f32,
}

impl Default for Config {
    fn default() -> Self {
        let preset = |name: &str, extruder, bed| Preset { name: name.into(), extruder, bed };
        Self {
            moonraker_url: "ws://127.0.0.1:7125/websocket".into(),
            xy_speed: 100.0,
            z_speed: 10.0,
            presets: vec![
                preset("PLA", 210.0, 60.0),
                preset("PETG", 240.0, 80.0),
                preset("ABS", 250.0, 100.0),
                preset("ASA", 260.0, 100.0),
            ],
        }
    }
}

impl Config {
    /// Kolejność: $KLIPPER_HELM_CONFIG, ~/.config/klipper-helm/config.toml, wartości domyślne.
    /// MOONRAKER_URL nadpisuje adres z pliku.
    pub fn load() -> Self {
        let mut cfg = match Self::path() {
            Some(path) if path.exists() => match std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|s| toml::from_str::<Config>(&s).map_err(Into::into))
            {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("Błąd konfiguracji {}: {e}", path.display());
                    Self::default()
                }
            },
            _ => Self::default(),
        };
        if let Ok(url) = std::env::var("MOONRAKER_URL") {
            cfg.moonraker_url = url;
        }
        cfg
    }

    fn path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("KLIPPER_HELM_CONFIG") {
            return Some(p.into());
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("klipper-helm").join("config.toml"))
    }
}
