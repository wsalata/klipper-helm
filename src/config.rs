use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_URL: &str = "ws://127.0.0.1:7125/websocket";
/// UI languages selectable in the settings ("auto" follows the system locale)
pub const LANGUAGES: [&str; 3] = ["auto", "en", "pl"];

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub moonraker_url: String,
    /// "auto", "en" or "pl"
    pub language: String,
    /// Manual XY movement speed [mm/s]
    pub xy_speed: f32,
    /// Manual Z movement speed [mm/s]
    pub z_speed: f32,
    /// Minutes without touches before the screen saver starts, 0 = never
    pub screensaver_minutes: u32,
    #[serde(rename = "preset")]
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Preset {
    pub name: String,
    pub extruder: f32,
    pub bed: f32,
}

/// Validation error as a code plus argument; the UI turns it into a translated message.
#[derive(Debug, PartialEq)]
pub struct ConfigError {
    pub code: &'static str,
    pub arg: String,
}

impl ConfigError {
    pub fn new(code: &'static str, arg: impl Into<String>) -> Self {
        Self { code, arg: arg.into() }
    }
}

impl Default for Config {
    fn default() -> Self {
        let preset = |name: &str, extruder, bed| Preset { name: name.into(), extruder, bed };
        Self {
            // Empty = not decided yet, see `load`
            moonraker_url: String::new(),
            language: "auto".into(),
            xy_speed: 100.0,
            z_speed: 10.0,
            screensaver_minutes: 5,
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
    /// File: $KLIPPER_HELM_CONFIG or ~/.config/klipper-helm/config.toml.
    /// Moonraker address: from the file, or if missing there, from MOONRAKER_URL or the default.
    /// This way an address saved from the panel wins over the variable in the service file.
    pub fn load() -> Self {
        let mut cfg = match Self::path() {
            Some(path) if path.exists() => match std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|s| toml::from_str::<Config>(&s).map_err(Into::into))
            {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("Config error in {}: {e}", path.display());
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

    /// Saves atomically (temporary file + rename) so a power loss cannot leave half a file.
    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| anyhow::anyhow!("cannot determine the config directory"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let url = self.moonraker_url.trim();
        let host = url.strip_prefix("ws://").or_else(|| url.strip_prefix("wss://"));
        if host.is_none_or(|h| h.is_empty() || h.contains(' ')) {
            return Err(ConfigError::new("url", ""));
        }
        for (axis, v) in [("XY", self.xy_speed), ("Z", self.z_speed)] {
            if !(0.1..=1000.0).contains(&v) {
                return Err(ConfigError::new("speed", axis));
            }
        }
        for p in &self.presets {
            if p.name.trim().is_empty() {
                return Err(ConfigError::new("preset-name", ""));
            }
            if !(0.0..=500.0).contains(&p.extruder) || !(0.0..=200.0).contains(&p.bed) {
                return Err(ConfigError::new("preset-temp", p.name.clone()));
            }
        }
        if !LANGUAGES.contains(&self.language.as_str()) {
            return Err(ConfigError::new("language", self.language.clone()));
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

    fn valid() -> Config {
        Config { moonraker_url: "ws://printer:7125/websocket".into(), ..Config::default() }
    }

    #[test]
    fn roundtrip_toml() {
        let cfg = valid();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("[[preset]]"));
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), cfg);
    }

    #[test]
    fn old_file_without_language_still_loads() {
        let cfg: Config = toml::from_str("moonraker_url = \"ws://p/websocket\"\n").unwrap();
        assert_eq!(cfg.language, "auto");
    }

    #[test]
    fn validation() {
        assert!(valid().validate().is_ok());
        let mut cfg = valid();
        cfg.moonraker_url = "http://printer".into();
        assert_eq!(cfg.validate().unwrap_err().code, "url");
        cfg.moonraker_url = "ws://".into();
        assert_eq!(cfg.validate().unwrap_err().code, "url");
        let mut cfg = valid();
        cfg.z_speed = 0.0;
        assert_eq!(cfg.validate().unwrap_err(), ConfigError::new("speed", "Z"));
        let mut cfg = valid();
        cfg.language = "de".into();
        assert_eq!(cfg.validate().unwrap_err().code, "language");
    }
}
