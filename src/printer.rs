//! Stan drukarki zbudowany z delt wysyłanych przez Moonrakera
//! i jego przekształcenie na struktury wyświetlane w UI.

use crate::{Conn, Fan, FileDetails, Heater, PrintJob, Position};
use serde_json::{Value, json};

/// Obiekty Klippera wykryte przez `printer.objects.list`.
#[derive(Default)]
pub struct Objects {
    pub heaters: Vec<String>,
    pub sensors: Vec<String>,
    pub fans: Vec<String>,
    pub macros: Vec<String>,
}

impl Objects {
    pub fn discover(list: &[Value]) -> Self {
        let mut o = Self::default();
        for name in list.iter().filter_map(Value::as_str) {
            let (kind, rest) = name.split_once(' ').unwrap_or((name, ""));
            match kind {
                "heater_bed" | "heater_generic" | "temperature_fan" => o.heaters.push(name.into()),
                k if k.starts_with("extruder") && !k.starts_with("extruder_stepper") => {
                    o.heaters.push(name.into())
                }
                "temperature_sensor" => o.sensors.push(name.into()),
                "fan" | "fan_generic" | "heater_fan" | "controller_fan" => o.fans.push(name.into()),
                "gcode_macro" if !rest.starts_with('_') => o.macros.push(rest.into()),
                _ => {}
            }
        }
        // Dysze najpierw, potem stół, potem reszta
        o.heaters.sort_by_key(|h| (heater_rank(h), h.clone()));
        o.fans.sort_by_key(|f| (f != "fan", f.clone()));
        o.macros.sort();
        o
    }

    pub fn subscription(&self) -> Value {
        let mut objects = serde_json::Map::new();
        for name in [
            "print_stats",
            "virtual_sdcard",
            "display_status",
            "toolhead",
            "gcode_move",
            "idle_timeout",
            "webhooks",
        ]
        .into_iter()
        .chain(self.heaters.iter().map(String::as_str))
        .chain(self.sensors.iter().map(String::as_str))
        .chain(self.fans.iter().map(String::as_str))
        {
            objects.insert(name.into(), Value::Null);
        }
        json!({ "objects": objects })
    }

    pub fn has(&self, heater: &str) -> bool {
        self.heaters.iter().any(|h| h == heater)
    }
}

fn heater_rank(name: &str) -> u8 {
    if name.starts_with("extruder") {
        0
    } else if name == "heater_bed" {
        1
    } else {
        2
    }
}

pub fn display_name(obj: &str) -> String {
    match obj {
        "extruder" => "Dysza".into(),
        "heater_bed" => "Stół".into(),
        "fan" => "Chłodzenie wydruku".into(),
        o if o.starts_with("extruder") => format!("Dysza {}", o.trim_start_matches("extruder")),
        o => {
            let name = o.split_once(' ').map_or(o, |(_, n)| n).replace('_', " ");
            let mut chars = name.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// G-code ustawiający temperaturę docelową danego grzałki.
pub fn set_temp_gcode(obj: &str, target: f32) -> String {
    let target = target.max(0.0).round();
    match obj.split_once(' ') {
        Some(("temperature_fan", name)) => {
            format!("SET_TEMPERATURE_FAN_TARGET TEMPERATURE_FAN={name} TARGET={target}")
        }
        Some((_, name)) => format!("SET_HEATER_TEMPERATURE HEATER={name} TARGET={target}"),
        None => format!("SET_HEATER_TEMPERATURE HEATER={obj} TARGET={target}"),
    }
}

pub fn set_fan_gcode(obj: &str, speed: f32) -> Option<String> {
    let speed = speed.clamp(0.0, 1.0);
    match obj.split_once(' ') {
        None if obj == "fan" => Some(format!("M106 S{}", (speed * 255.0).round())),
        Some(("fan_generic", name)) => Some(format!("SET_FAN_SPEED FAN={name} SPEED={speed:.2}")),
        _ => None,
    }
}

/// Rekurencyjne scalanie: Moonraker wysyła tylko zmienione pola.
pub fn merge(dst: &mut Value, src: &Value) {
    match (dst, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                merge(d.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (d, s) => *d = s.clone(),
    }
}

/// Wszystko, co trafia do UI po każdej aktualizacji stanu.
pub struct Snapshot {
    pub conn: Conn,
    pub status_label: String,
    pub status_message: String,
    pub top_temps: String,
    pub heaters: Vec<Heater>,
    pub sensors: Vec<Heater>,
    pub fans: Vec<Fan>,
    pub job: PrintJob,
    pub position: Position,
    pub extruder_temp: f32,
    pub has_extruder: bool,
}

#[derive(Default)]
pub struct PrinterModel {
    pub status: Value,
    pub objects: Objects,
    pub klippy_state: String,
    pub klippy_message: String,
    pub subscribed: bool,
    /// Metadane aktualnie drukowanego pliku: (ścieżka, metadane lub Null)
    pub job_meta: Option<(String, Value)>,
}

impl PrinterModel {
    fn f(&self, path: &str) -> Option<f64> {
        self.status.pointer(path).and_then(Value::as_f64)
    }

    fn s(&self, path: &str) -> &str {
        self.status.pointer(path).and_then(Value::as_str).unwrap_or("")
    }

    pub fn filename(&self) -> &str {
        self.s("/print_stats/filename")
    }

    fn meta(&self, key: &str) -> Option<f64> {
        let (path, meta) = self.job_meta.as_ref()?;
        (path == self.filename()).then(|| meta.get(key)?.as_f64())?
    }

    pub fn conn(&self) -> Conn {
        let webhooks = self.s("/webhooks/state");
        if self.subscribed && (webhooks.is_empty() || webhooks == "ready") {
            Conn::Ready
        } else {
            Conn::NotReady
        }
    }

    fn status_label(&self) -> String {
        if self.conn() != Conn::Ready {
            return match self.klippy_state.as_str() {
                "startup" => "Klipper się uruchamia",
                "shutdown" => "Klipper wyłączony",
                "error" => "Błąd Klippera",
                "ready" => "Łączenie z Klipperem...",
                _ => "Klipper niedostępny",
            }
            .into();
        }
        match self.s("/print_stats/state") {
            "printing" => "Drukuje",
            "paused" => "Pauza",
            "complete" => "Zakończono",
            "cancelled" => "Anulowano",
            "error" => "Błąd druku",
            _ if self.s("/idle_timeout/state") == "Printing" => "Zajęta",
            _ => "Gotowa",
        }
        .into()
    }

    fn heater(&self, obj: &str, settable: bool) -> Heater {
        let p = |field: &str| self.f(&format!("/{obj}/{field}")).unwrap_or(0.0) as f32;
        Heater {
            id: obj.into(),
            name: display_name(obj).into(),
            temp: p("temperature"),
            target: p("target"),
            power: if obj.starts_with("temperature_fan") { p("speed") } else { p("power") },
            settable,
        }
    }

    fn job(&self) -> PrintJob {
        let state = self.s("/print_stats/state");
        let duration = self.f("/print_stats/print_duration").unwrap_or(0.0);
        let progress = self
            .f("/display_status/progress")
            .filter(|p| *p > 0.0)
            .or_else(|| self.f("/virtual_sdcard/progress"))
            .unwrap_or(0.0);

        // Powyżej 10% ufamy postępowi w pliku, wcześniej szacunkowi slicera
        let file_eta = (progress > 0.0).then(|| duration / progress - duration);
        let slicer_eta = self.meta("estimated_time").map(|t| (t - duration).max(0.0));
        let remaining = match (file_eta, slicer_eta) {
            (Some(f), _) if progress > 0.1 => Some(f),
            (f, s) => s.or(f),
        };

        let layer = match (
            self.f("/print_stats/info/current_layer"),
            self.f("/print_stats/info/total_layer"),
        ) {
            (Some(c), Some(t)) => format!("{c:.0} / {t:.0}"),
            _ => self.layer_from_meta().unwrap_or_else(|| "-".into()),
        };

        let active = matches!(state, "printing" | "paused");
        let finished = matches!(state, "complete" | "cancelled" | "error");
        PrintJob {
            state: state.into(),
            filename: if active || finished { self.filename().into() } else { "".into() },
            progress: if state == "complete" { 1.0 } else { progress as f32 },
            elapsed: fmt_duration(duration).into(),
            remaining: remaining.filter(|_| active).map_or_else(|| "-".into(), fmt_duration).into(),
            layer: layer.into(),
            filament: format!("{:.2} m", self.f("/print_stats/filament_used").unwrap_or(0.0) / 1000.0).into(),
            message: self.s("/print_stats/message").into(),
            speed_factor: (self.f("/gcode_move/speed_factor").unwrap_or(1.0) * 100.0).round() as i32,
            flow_factor: (self.f("/gcode_move/extrude_factor").unwrap_or(1.0) * 100.0).round() as i32,
            z_offset: format!("{:+.3}", self.f("/gcode_move/homing_origin/2").unwrap_or(0.0)).into(),
        }
    }

    fn layer_from_meta(&self) -> Option<String> {
        let lh = self.meta("layer_height").filter(|h| *h > 0.0)?;
        let first = self.meta("first_layer_height").unwrap_or(lh);
        let height = self.meta("object_height")?;
        let z = self.f("/gcode_move/gcode_position/2")?;
        let total = ((height - first) / lh).round() + 1.0;
        let current = (((z - first) / lh).round() + 1.0).clamp(1.0, total);
        Some(format!("{current:.0} / {total:.0}"))
    }

    pub fn snapshot(&self) -> Snapshot {
        let heaters: Vec<Heater> = self.objects.heaters.iter().map(|h| self.heater(h, true)).collect();
        let sensors = self.objects.sensors.iter().map(|s| self.heater(s, false)).collect();
        let fans = self
            .objects
            .fans
            .iter()
            .map(|f| Fan {
                id: f.as_str().into(),
                name: display_name(f).into(),
                speed: self.f(&format!("/{f}/speed")).unwrap_or(0.0) as f32,
                settable: set_fan_gcode(f, 0.0).is_some(),
            })
            .collect();

        let homed = self.s("/toolhead/homed_axes");
        let axis = |i: usize| {
            self.f(&format!("/gcode_move/gcode_position/{i}"))
                .map_or_else(|| "-".into(), |v| format!("{v:.2}"))
        };
        let position = Position {
            x: axis(0).into(),
            y: axis(1).into(),
            z: axis(2).into(),
            homed_x: homed.contains('x'),
            homed_y: homed.contains('y'),
            homed_z: homed.contains('z'),
        };

        let top_temps = heaters
            .iter()
            .filter(|h| h.id == "extruder" || h.id == "heater_bed")
            .map(|h| format!("{} {:.0}/{:.0}°", h.name, h.temp, h.target))
            .collect::<Vec<_>>()
            .join("   ");

        let extruder = heaters.iter().find(|h| h.id == "extruder");
        Snapshot {
            conn: self.conn(),
            status_label: self.status_label(),
            status_message: if self.conn() == Conn::Ready {
                String::new()
            } else {
                self.klippy_message.clone()
            },
            top_temps,
            extruder_temp: extruder.map_or(0.0, |h| h.temp),
            has_extruder: extruder.is_some(),
            heaters,
            sensors,
            fans,
            job: self.job(),
            position,
        }
    }
}

pub fn fmt_duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}h {m:02}m") } else { format!("{m}m {s:02}s") }
}

fn fmt_size(bytes: f64) -> String {
    match bytes {
        b if b >= 1e6 => format!("{:.1} MB", b / 1e6),
        b if b >= 1e3 => format!("{:.0} kB", b / 1e3),
        b => format!("{b:.0} B"),
    }
}

fn fmt_time(epoch: f64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(epoch as i64, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Wiersz listy plików. Bez obrazka, bo `slint::Image` nie może opuścić wątku UI.
pub struct FileRow {
    pub path: String,
    pub size: String,
    pub modified: String,
    /// Surowy czas modyfikacji, do rozpoznania nowej wersji pliku
    pub stamp: Value,
}

/// Lista plików z `server.files.list`, najnowsze najpierw.
pub fn file_list(result: &Value) -> Vec<FileRow> {
    let mut files: Vec<(f64, FileRow)> = result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let path = f.get("path").or_else(|| f.get("filename"))?.as_str()?;
            let stamp = f.get("modified").cloned().unwrap_or(Value::Null);
            let modified = stamp.as_f64().unwrap_or(0.0);
            Some((
                modified,
                FileRow {
                    path: path.into(),
                    size: fmt_size(f.get("size").and_then(Value::as_f64).unwrap_or(0.0)),
                    modified: fmt_time(modified),
                    stamp,
                },
            ))
        })
        .collect();
    files.sort_by(|a, b| b.0.total_cmp(&a.0));
    files.into_iter().map(|(_, f)| f).collect()
}

pub fn file_details(path: &str, meta: &Value) -> FileDetails {
    let num = |k: &str| meta.get(k).and_then(Value::as_f64);
    FileDetails {
        path: path.into(),
        estimated: num("estimated_time").map_or_else(|| "-".into(), fmt_duration).into(),
        filament: num("filament_total")
            .map_or_else(|| "-".into(), |mm| format!("{:.2} m", mm / 1000.0))
            .into(),
        slicer: match (meta.get("slicer").and_then(Value::as_str), meta.get("slicer_version").and_then(Value::as_str)) {
            (Some(s), Some(v)) => format!("{s} {v}"),
            (Some(s), None) => s.into(),
            _ => "-".into(),
        }
        .into(),
        layer_height: num("layer_height").map_or_else(|| "-".into(), |h| format!("{h} mm")).into(),
    }
}
