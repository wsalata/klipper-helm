use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_URL: &str = "ws://127.0.0.1:7125/websocket";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Preset {
    pub name: String,
    pub extruder: f32,
    pub bed: f32,
}

impl Default for Config {
    fn default() -> Self {
        let preset = |name: &str, extruder, bed| Preset { name: name.into(), extruder, bed };
        Self {
            // Puste = jeszcze nieustalone, patrz `load`
            moonraker_url: String::new(),
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
    /// Plik: $KLIPPER_HELM_CONFIG albo ~/.config/klipper-helm/config.toml.
    /// Adres Moonrakera: z pliku, a gdy go tam nie ma, z MOONRAKER_URL lub domyślny.
    /// Dzięki temu adres zapisany z panelu wygrywa ze zmienną z pliku usługi.
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
        if cfg.moonraker_url.is_empty() {
            cfg.moonraker_url = std::env::var("MOONRAKER_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        }
        cfg
    }

    /// Zapisuje atomowo (plik tymczasowy + rename), żeby zanik zasilania nie zostawił połowy pliku.
    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| anyhow::anyhow!("nie można ustalić katalogu konfiguracji"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// Opis błędu, jeśli ustawienia są niepoprawne.
    pub fn validate(&self) -> Result<(), String> {
        let url = self.moonraker_url.trim();
        let host = url.strip_prefix("ws://").or_else(|| url.strip_prefix("wss://"));
        if host.is_none_or(|h| h.is_empty() || h.contains(' ')) {
            return Err("Adres Moonrakera musi zaczynać się od ws:// lub wss:// i nie może mieć spacji".into());
        }
        for (name, v) in [("XY", self.xy_speed), ("Z", self.z_speed)] {
            if !(0.1..=1000.0).contains(&v) {
                return Err(format!("Prędkość {name} musi być w zakresie 0,1-1000 mm/s"));
            }
        }
        for p in &self.presets {
            if p.name.trim().is_empty() {
                return Err("Preset musi mieć nazwę".into());
            }
            if !(0.0..=500.0).contains(&p.extruder) || !(0.0..=200.0).contains(&p.bed) {
                return Err(format!("Preset {}: temperatura poza zakresem", p.name));
            }
        }
        Ok(())
    }

    pub fn path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("KLIPPER_HELM_CONFIG") {
            return Some(p.into());
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("klipper-helm").join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_toml() {
        let mut cfg = Config::default();
        cfg.moonraker_url = "ws://printer:7125/websocket".into();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("[[preset]]"));
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), cfg);
    }

    #[test]
    fn validation() {
        let mut cfg = Config::default();
        cfg.moonraker_url = "ws://printer:7125/websocket".into();
        assert!(cfg.validate().is_ok());
        cfg.moonraker_url = "http://printer".into();
        assert!(cfg.validate().is_err());
        cfg.moonraker_url = "ws://".into();
        assert!(cfg.validate().is_err());
    }
}
