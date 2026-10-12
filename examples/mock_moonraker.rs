//! Udawany Moonraker do pracy nad UI bez drukarki.
//! Uruchom: `cargo run --example mock_moonraker`, potem
//! `MOONRAKER_URL=ws://127.0.0.1:7125/websocket cargo run`.
//! Symuluje nagrzewanie, wydruk w toku, odpowiedzi na G-code i miniaturki plików
//! (serwowane przez HTTP na tym samym porcie, jak w prawdziwym Moonrakerze).

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// (ścieżka, czas modyfikacji, rozmiar, czy ma miniaturki)
const FILES: &[(&str, f64, u64, bool)] = &[
    ("benchy_0.2mm_PLA.gcode", 1760200000.0, 4_200_000, true),
    ("calibration/cube_20mm.gcode", 1760100000.0, 820_000, true),
    ("voron_logo_PETG.gcode", 1759900000.0, 2_500_000, true),
    ("stary_plik_bez_miniaturki.gcode", 1750000000.0, 150_000, false),
];
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:7125".into());
    let listener = TcpListener::bind(&addr).await?;
    println!("Mock Moonraker na ws://{addr}/websocket");
    loop {
        let (stream, peer) = listener.accept().await?;
        println!("Klient: {peer}");
        tokio::spawn(async move {
            if let Err(e) = client(stream).await {
                println!("Klient {peer}: {e}");
            }
        });
    }
}

struct Sim {
    t: f64,
    ext_target: f64,
    ext: f64,
    bed_target: f64,
    bed: f64,
    progress: f64,
    state: &'static str,
    fan: f64,
    speed_factor: f64,
}

impl Sim {
    fn step(&mut self, dt: f64) {
        self.t += dt;
        self.ext += (self.ext_target.max(22.0) - self.ext) * 0.15;
        self.bed += (self.bed_target.max(22.0) - self.bed) * 0.08;
        if self.state == "printing" {
            self.progress = (self.progress + 0.002 * self.speed_factor).min(1.0);
            if self.progress >= 1.0 {
                self.state = "complete";
            }
        }
    }

    fn status(&self) -> Value {
        let power = |cur: f64, target: f64| if target > 0.0 { ((target - cur) / 30.0).clamp(0.1, 1.0) } else { 0.0 };
        let z = 0.2 + self.progress * 24.0;
        json!({
            "extruder": { "temperature": self.ext, "target": self.ext_target, "power": power(self.ext, self.ext_target) },
            "heater_bed": { "temperature": self.bed, "target": self.bed_target, "power": power(self.bed, self.bed_target) },
            "temperature_sensor raspberry_pi": { "temperature": 48.3 + (self.t / 7.0).sin() },
            "fan": { "speed": self.fan },
            "heater_fan hotend_fan": { "speed": if self.ext > 50.0 { 1.0 } else { 0.0 } },
            "print_stats": {
                "state": self.state,
                "filename": if self.state == "standby" { "" } else { "benchy_0.2mm_PLA.gcode" },
                "print_duration": self.progress * 5400.0,
                "filament_used": self.progress * 5300.0,
                "message": "",
                "info": { "current_layer": (z / 0.2).round(), "total_layer": 121 },
            },
            "virtual_sdcard": { "progress": self.progress },
            "display_status": { "progress": self.progress },
            "toolhead": { "homed_axes": "xyz", "position": [117.5, 112.3, z, 0.0] },
            "gcode_move": {
                "gcode_position": [117.5, 112.3, z, 0.0],
                "homing_origin": [0.0, 0.0, -0.025, 0.0],
                "speed_factor": self.speed_factor,
                "extrude_factor": 1.0,
            },
            "idle_timeout": { "state": if self.state == "printing" { "Printing" } else { "Idle" } },
            "webhooks": { "state": "ready", "state_message": "Printer is ready" },
        })
    }
}

async fn client(stream: TcpStream) -> anyhow::Result<()> {
    let mut head = [0u8; 32];
    let n = stream.peek(&mut head).await?;
    if head[..n].starts_with(b"GET /server/files/") {
        return serve_thumb(stream).await;
    }
    let ws = accept_async(stream).await?;
    let (mut tx, mut rx) = ws.split();
    let mut sim = Sim {
        t: 0.0,
        ext_target: 210.0,
        ext: 25.0,
        bed_target: 60.0,
        bed: 25.0,
        progress: 0.37,
        state: "printing",
        fan: 1.0,
        speed_factor: 1.0,
    };
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                sim.step(0.5);
                let msg = json!({ "jsonrpc": "2.0", "method": "notify_status_update", "params": [sim.status(), sim.t] });
                tx.send(Message::text(msg.to_string())).await?;
            }
            msg = rx.next() => {
                let Some(msg) = msg else { return Ok(()) };
                let Message::Text(text) = msg? else { continue };
                let req: Value = serde_json::from_str(text.as_str())?;
                let id = req["id"].clone();
                let method = req["method"].as_str().unwrap_or("");
                let mut notifications = Vec::new();
                let result = match method {
                    "printer.info" => json!({
                        "state": "ready", "state_message": "Printer is ready",
                        "hostname": "voron-mock", "software_version": "v0.13.0-mock",
                    }),
                    "printer.objects.list" => json!({ "objects": [
                        "webhooks", "configfile", "extruder", "heater_bed", "temperature_sensor raspberry_pi",
                        "fan", "heater_fan hotend_fan", "print_stats", "virtual_sdcard", "display_status",
                        "toolhead", "gcode_move", "idle_timeout",
                        "gcode_macro PRINT_START", "gcode_macro PRINT_END", "gcode_macro LOAD_FILAMENT",
                        "gcode_macro UNLOAD_FILAMENT", "gcode_macro _HELPER", "gcode_macro BED_MESH_CALIBRATE_FAST",
                        "gcode_macro PARK", "gcode_macro M600",
                    ]}),
                    "printer.objects.subscribe" => json!({ "status": sim.status(), "eventtime": sim.t }),
                    "server.gcode_store" => json!({ "gcode_store": [
                        { "message": "G28", "time": 0.0, "type": "command" },
                        { "message": "// Homing complete", "time": 0.0, "type": "response" },
                        { "message": "ok", "time": 0.0, "type": "response" },
                    ]}),
                    "server.files.list" => FILES
                        .iter()
                        .map(|(path, modified, size, _)| json!({ "path": path, "modified": modified, "size": size }))
                        .collect(),
                    "server.files.metadata" => metadata(req["params"]["filename"].as_str().unwrap_or("")),
                    "printer.gcode.script" => {
                        let script = req["params"]["script"].as_str().unwrap_or("");
                        for line in script.lines() {
                            let mut words = line.split_whitespace();
                            match words.next() {
                                Some("SET_HEATER_TEMPERATURE") => {
                                    let arg = |k: &str| line.split_whitespace().find_map(|w| w.strip_prefix(k)).unwrap_or("");
                                    let target: f64 = arg("TARGET=").parse().unwrap_or(0.0);
                                    match arg("HEATER=") {
                                        "extruder" => sim.ext_target = target,
                                        "heater_bed" => sim.bed_target = target,
                                        _ => {}
                                    }
                                }
                                Some("TURN_OFF_HEATERS") => { sim.ext_target = 0.0; sim.bed_target = 0.0; }
                                Some("M106") => {
                                    sim.fan = words.next().and_then(|w| w.strip_prefix('S')).and_then(|v| v.parse::<f64>().ok()).unwrap_or(255.0) / 255.0;
                                }
                                Some("M220") => {
                                    sim.speed_factor = words.next().and_then(|w| w.strip_prefix('S')).and_then(|v| v.parse::<f64>().ok()).unwrap_or(100.0) / 100.0;
                                }
                                Some("FAIL") => {
                                    let err = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": 400, "message": "Unknown command:\"FAIL\"" } });
                                    tx.send(Message::text(err.to_string())).await?;
                                    continue;
                                }
                                _ => {}
                            }
                            notifications.push(json!({ "jsonrpc": "2.0", "method": "notify_gcode_response", "params": [format!("// mock: {line}")] }));
                        }
                        json!("ok")
                    }
                    "printer.print.pause" => { sim.state = "paused"; json!("ok") }
                    "printer.print.resume" => { sim.state = "printing"; json!("ok") }
                    "printer.print.cancel" => { sim.state = "cancelled"; json!("ok") }
                    "printer.print.start" => { sim.state = "printing"; sim.progress = 0.0; json!("ok") }
                    _ => json!("ok"),
                };
                for n in notifications {
                    tx.send(Message::text(n.to_string())).await?;
                }
                let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
                tx.send(Message::text(resp.to_string())).await?;
            }
        }
    }
}

fn metadata(filename: &str) -> Value {
    let (modified, has_thumbs) = FILES
        .iter()
        .find(|f| f.0 == filename)
        .map_or((0.0, false), |f| (f.1, f.3));
    let stem = filename.rsplit('/').next().unwrap_or(filename).trim_end_matches(".gcode");
    let thumbnails: Vec<Value> = if has_thumbs {
        [32, 300]
            .iter()
            .map(|s| json!({ "width": s, "height": s, "size": 1000, "relative_path": format!(".thumbs/{stem}-{s}x{s}.png") }))
            .collect()
    } else {
        Vec::new()
    };
    json!({
        "modified": modified, "estimated_time": 5400.0, "filament_total": 5300.0, "layer_height": 0.2,
        "first_layer_height": 0.2, "object_height": 24.2, "slicer": "OrcaSlicer", "slicer_version": "2.3.0",
        "thumbnails": thumbnails,
    })
}

/// GET /server/files/gcodes/.../.thumbs/<nazwa>-<W>x<H>.png
async fn serve_thumb(mut stream: TcpStream) -> anyhow::Result<()> {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        request.extend_from_slice(&buf[..n]);
    }
    let request = String::from_utf8_lossy(&request);
    let path = request.split_whitespace().nth(1).unwrap_or("");
    println!("HTTP {path}");
    let name = path.rsplit('/').next().unwrap_or("").trim_end_matches(".png");
    let (stem, dims) = name.rsplit_once('-').unwrap_or((name, "64x64"));
    let size = dims.split('x').next().and_then(|w| w.parse().ok()).unwrap_or(64u32).clamp(8, 512);

    let body = render_thumb(stem, size)?;
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Udawany render wydruku: kolorowa kula na podstawce, przezroczyste tło jak w slicerach.
fn render_thumb(stem: &str, size: u32) -> anyhow::Result<Vec<u8>> {
    let hash = stem.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hue = (hash % 360) as f32;
    let color = |light: f32| hsl(hue, 0.7, light);

    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
            let (dx, dy) = (u - 0.5, v - 0.47);
            let px = if dx * dx + dy * dy < 0.31 * 0.31 {
                // Światło z lewej góry
                let shade = 0.65 - ((u - 0.38).powi(2) + (v - 0.33).powi(2)).sqrt() * 0.8;
                color(shade.clamp(0.2, 0.75))
            } else if (0.78..0.9).contains(&v) && (0.14..0.86).contains(&u) {
                color(0.28)
            } else {
                [0, 0, 0, 0]
            };
            pixels.extend_from_slice(&px);
        }
    }

    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    Ok(out)
}

fn hsl(h: f32, s: f32, l: f32) -> [u8; 4] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m) * 255.0).round() as u8;
    [to(r), to(g), to(b), 255]
}
