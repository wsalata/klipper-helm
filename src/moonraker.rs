//! Klient JSON-RPC Moonrakera po WebSockecie z automatycznym wznawianiem połączenia.

use crate::config::Config;
use crate::printer::{self, Objects, PrinterModel};
use crate::thumbs::{Size, Thumbs};
use crate::{AppWindow, Conn, ConsoleLine, bridge};
use anyhow::bail;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use slint::Weak;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const RETRY: Duration = Duration::from_secs(2);
/// Moonraker wysyła notify_proc_stat_update co sekundę, więc cisza oznacza zerwane połączenie.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(15);
/// Dla tylu najnowszych plików pobieramy miniaturki na listę
const LIST_THUMBS: usize = 100;

pub enum Command {
    Gcode(String),
    /// G-code wpisany w konsoli (echo w historii)
    Console(String),
    SetTemp(String, f32),
    SetFan(String, f32),
    Jog(String, f32),
    Extrude(f32, f32),
    Pause,
    Resume,
    Cancel,
    EmergencyStop,
    Restart,
    FirmwareRestart,
    RefreshFiles,
    FileDetails(String),
    StartPrint(String),
    HostReboot,
    HostShutdown,
}

/// Na co czeka dane zapytanie, żeby wiedzieć, jak obsłużyć odpowiedź.
enum Pending {
    Ignore,
    Action,
    Gcode,
    PrinterInfo,
    ObjectsList,
    Subscribe,
    GcodeStore,
    FileList,
    FileMeta(String),
    JobMeta(String),
    /// Metadane potrzebne tylko do miniaturki na liście plików
    ListMeta(String),
}

enum Exit {
    Quit,
    Lost,
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn run(cfg: Config, ui: Weak<AppWindow>, mut rx: UnboundedReceiver<Command>) {
    let mut thumbs = Thumbs::new(&cfg.moonraker_url);
    loop {
        bridge::conn(&ui, Conn::Connecting, "Łączenie...", cfg.moonraker_url.clone());
        match connect_async(cfg.moonraker_url.as_str()).await {
            Ok((ws, _)) => {
                let mut session = Session::new(ws, &cfg, ui.clone(), &mut thumbs);
                match session.run(&mut rx).await {
                    Ok(Exit::Quit) => return,
                    Ok(Exit::Lost) => eprintln!("Moonraker: połączenie zamknięte"),
                    Err(e) => eprintln!("Moonraker: {e}"),
                }
            }
            Err(e) => eprintln!("Moonraker {}: {e}", cfg.moonraker_url),
        }
        bridge::conn(
            &ui,
            Conn::Disconnected,
            "Brak połączenia",
            format!("Nie można połączyć z Moonrakerem ({})", cfg.moonraker_url),
        );

        let deadline = tokio::time::Instant::now() + RETRY;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                cmd = rx.recv() => match cmd {
                    None => return,
                    Some(_) => bridge::toast(&ui, "Brak połączenia z Moonrakerem"),
                },
            }
        }
    }
}

struct Session<'a> {
    ws: Ws,
    cfg: &'a Config,
    thumbs: &'a mut Thumbs,
    ui: Weak<AppWindow>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    model: PrinterModel,
    discovering: bool,
    job_meta_requested: String,
    last_rx: Instant,
}

impl<'a> Session<'a> {
    fn new(ws: Ws, cfg: &'a Config, ui: Weak<AppWindow>, thumbs: &'a mut Thumbs) -> Self {
        Self {
            ws,
            cfg,
            thumbs,
            ui,
            next_id: 1,
            pending: HashMap::new(),
            model: PrinterModel::default(),
            discovering: false,
            job_meta_requested: String::new(),
            last_rx: Instant::now(),
        }
    }

    async fn call(&mut self, method: &str, params: Value, pending: Pending) -> anyhow::Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.ws.send(Message::text(msg.to_string())).await?;
        Ok(())
    }

    async fn gcode(&mut self, script: String) -> anyhow::Result<()> {
        self.call("printer.gcode.script", json!({ "script": script }), Pending::Gcode).await
    }

    async fn run(&mut self, rx: &mut UnboundedReceiver<Command>) -> anyhow::Result<Exit> {
        self.call(
            "server.connection.identify",
            json!({
                "client_name": "klipper-helm",
                "version": env!("CARGO_PKG_VERSION"),
                "type": "display",
                "url": "https://github.com/",
            }),
            Pending::Ignore,
        )
        .await?;
        self.call("printer.info", json!({}), Pending::PrinterInfo).await?;
        self.call("server.gcode_store", json!({ "count": 100 }), Pending::GcodeStore).await?;
        self.call("server.files.list", json!({ "root": "gcodes" }), Pending::FileList).await?;
        self.push_state();

        let mut tick = tokio::time::interval(RETRY);
        loop {
            tokio::select! {
                msg = self.ws.next() => {
                    let Some(msg) = msg else { return Ok(Exit::Lost) };
                    self.last_rx = Instant::now();
                    match msg? {
                        Message::Text(text) => self.on_text(text.as_str()).await?,
                        Message::Close(_) => return Ok(Exit::Lost),
                        _ => {}
                    }
                }
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { return Ok(Exit::Quit) };
                    self.on_command(cmd).await?;
                }
                _ = tick.tick() => {
                    if self.last_rx.elapsed() > SILENCE_TIMEOUT {
                        bail!("brak danych od {}s", SILENCE_TIMEOUT.as_secs());
                    }
                    // Klipper niegotowy: odpytujemy, aż wstanie
                    if !self.model.subscribed && !self.discovering {
                        self.call("printer.info", json!({}), Pending::PrinterInfo).await?;
                    }
                }
            }
        }
    }

    async fn on_text(&mut self, text: &str) -> anyhow::Result<()> {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Moonraker: niepoprawny JSON: {e}");
                return Ok(());
            }
        };
        if let Some(method) = v.get("method").and_then(Value::as_str) {
            return self.on_notify(method, &v["params"]).await;
        }
        let Some(pending) = v.get("id").and_then(Value::as_u64).and_then(|id| self.pending.remove(&id))
        else {
            return Ok(());
        };
        match v.get("error") {
            Some(err) => {
                let msg = err.get("message").and_then(Value::as_str).unwrap_or("nieznany błąd");
                self.on_error(pending, msg.to_string());
                Ok(())
            }
            None => self.on_result(pending, &v["result"]).await,
        }
    }

    async fn on_notify(&mut self, method: &str, params: &Value) -> anyhow::Result<()> {
        match method {
            "notify_status_update" => {
                if self.model.subscribed {
                    printer::merge(&mut self.model.status, &params[0]);
                    self.push_state();
                    self.request_job_meta().await?;
                }
            }
            "notify_gcode_response" => {
                if let Some(line) = params[0].as_str() {
                    bridge::console(&self.ui, vec![bridge::console_line(line)]);
                }
            }
            "notify_klippy_ready" => {
                self.model.subscribed = false;
                self.call("printer.info", json!({}), Pending::PrinterInfo).await?;
            }
            "notify_klippy_shutdown" | "notify_klippy_disconnected" => {
                self.model.subscribed = false;
                self.discovering = false;
                self.model.klippy_state =
                    if method == "notify_klippy_shutdown" { "shutdown" } else { "disconnected" }.into();
                self.push_state();
                self.call("printer.info", json!({}), Pending::PrinterInfo).await?;
            }
            "notify_filelist_changed" => {
                self.call("server.files.list", json!({ "root": "gcodes" }), Pending::FileList).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn on_result(&mut self, pending: Pending, result: &Value) -> anyhow::Result<()> {
        match pending {
            Pending::Ignore | Pending::Action | Pending::Gcode => {}
            Pending::PrinterInfo => {
                let s = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                self.model.klippy_state = s("state");
                self.model.klippy_message = s("state_message").trim().to_string();
                bridge::info(&self.ui, s("hostname"), s("software_version"));
                if self.model.klippy_state == "ready" && !self.model.subscribed && !self.discovering {
                    self.discovering = true;
                    self.call("printer.objects.list", json!({}), Pending::ObjectsList).await?;
                }
                self.push_state();
            }
            Pending::ObjectsList => {
                let list = result.get("objects").and_then(Value::as_array).cloned().unwrap_or_default();
                self.model.objects = Objects::discover(&list);
                bridge::macros(&self.ui, self.model.objects.macros.clone());
                let sub = self.model.objects.subscription();
                self.call("printer.objects.subscribe", sub, Pending::Subscribe).await?;
            }
            Pending::Subscribe => {
                self.model.status = result.get("status").cloned().unwrap_or(Value::Null);
                self.model.subscribed = true;
                self.discovering = false;
                self.push_state();
                self.request_job_meta().await?;
            }
            Pending::GcodeStore => {
                let lines = result
                    .get("gcode_store")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        let msg = e.get("message")?.as_str()?;
                        Some(if e.get("type").and_then(Value::as_str) == Some("command") {
                            ConsoleLine { text: msg.into(), kind: 1 }
                        } else {
                            bridge::console_line(msg)
                        })
                    })
                    .collect();
                bridge::console(&self.ui, lines);
            }
            Pending::FileList => {
                let files = printer::file_list(result);
                let missing: Vec<String> = files
                    .iter()
                    .take(LIST_THUMBS)
                    .filter(|f| !self.thumbs.known(&f.path, &f.stamp))
                    .map(|f| f.path.clone())
                    .collect();
                bridge::files(&self.ui, files);
                for path in missing {
                    let params = json!({ "filename": path });
                    self.call("server.files.metadata", params, Pending::ListMeta(path)).await?;
                }
            }
            Pending::ListMeta(path) => self.fetch_thumb(&path, result, Size::Small),
            Pending::FileMeta(path) => {
                bridge::details(&self.ui, printer::file_details(&path, result));
                self.fetch_thumb(&path, result, Size::Large);
            }
            Pending::JobMeta(path) => {
                self.fetch_thumb(&path, result, Size::Large);
                self.model.job_meta = Some((path, result.clone()));
                self.push_state();
            }
        }
        Ok(())
    }

    fn on_error(&mut self, pending: Pending, msg: String) {
        match pending {
            Pending::PrinterInfo => {
                self.model.klippy_state = "disconnected".into();
                self.model.klippy_message = msg;
                self.push_state();
            }
            Pending::ObjectsList | Pending::Subscribe => {
                eprintln!("Moonraker: inicjalizacja nieudana: {msg}");
                self.discovering = false;
            }
            Pending::Gcode => {
                bridge::console(&self.ui, vec![ConsoleLine { text: format!("!! {msg}").into(), kind: 2 }]);
                bridge::toast(&self.ui, msg);
            }
            Pending::FileMeta(path) => {
                bridge::details(&self.ui, printer::file_details(&path, &Value::Null));
            }
            Pending::JobMeta(path) => self.model.job_meta = Some((path, Value::Null)),
            Pending::ListMeta(_) => {}
            Pending::Ignore | Pending::GcodeStore | Pending::FileList => {
                eprintln!("Moonraker: {msg}");
            }
            Pending::Action => bridge::toast(&self.ui, msg),
        }
    }

    async fn on_command(&mut self, cmd: Command) -> anyhow::Result<()> {
        let action = |m: &str| (m.to_string(), json!({}));
        let (method, params) = match cmd {
            Command::Gcode(g) => return self.gcode(g).await,
            Command::Console(g) => {
                bridge::console(&self.ui, vec![ConsoleLine { text: g.as_str().into(), kind: 1 }]);
                return self.gcode(g).await;
            }
            Command::SetTemp(heater, target) => {
                // Presety ustawiają też stół, którego drukarka może nie mieć
                if !self.model.objects.has(&heater) {
                    return Ok(());
                }
                return self.gcode(printer::set_temp_gcode(&heater, target)).await;
            }
            Command::SetFan(fan, speed) => match printer::set_fan_gcode(&fan, speed) {
                Some(g) => return self.gcode(g).await,
                None => return Ok(()),
            },
            Command::Jog(axis, dist) => {
                let speed = if axis == "Z" { self.cfg.z_speed } else { self.cfg.xy_speed };
                return self.gcode(format!("G91\nG1 {axis}{dist} F{:.0}\nG90", speed * 60.0)).await;
            }
            Command::Extrude(mm, speed) => {
                return self.gcode(format!("M83\nG1 E{mm} F{:.0}", speed * 60.0)).await;
            }
            Command::Pause => action("printer.print.pause"),
            Command::Resume => action("printer.print.resume"),
            Command::Cancel => action("printer.print.cancel"),
            Command::EmergencyStop => action("printer.emergency_stop"),
            Command::Restart => action("printer.restart"),
            Command::FirmwareRestart => action("printer.firmware_restart"),
            Command::HostReboot => action("machine.reboot"),
            Command::HostShutdown => action("machine.shutdown"),
            Command::StartPrint(file) => ("printer.print.start".into(), json!({ "filename": file })),
            Command::RefreshFiles => {
                return self.call("server.files.list", json!({ "root": "gcodes" }), Pending::FileList).await;
            }
            Command::FileDetails(file) => {
                let params = json!({ "filename": file });
                return self.call("server.files.metadata", params, Pending::FileMeta(file)).await;
            }
        };
        self.call(&method, params, Pending::Action).await
    }

    /// Pobiera metadane drukowanego pliku (szacowany czas, wysokość warstwy).
    async fn request_job_meta(&mut self) -> anyhow::Result<()> {
        let file = self.model.filename().to_string();
        if file.is_empty() || file == self.job_meta_requested {
            return Ok(());
        }
        self.job_meta_requested = file.clone();
        self.call("server.files.metadata", json!({ "filename": file }), Pending::JobMeta(file)).await
    }

    fn fetch_thumb(&mut self, path: &str, meta: &Value, size: Size) {
        let ui = self.ui.clone();
        let file = path.to_string();
        self.thumbs.fetch(path, meta, size, move |pixels| bridge::thumbnail(&ui, file, size, pixels));
    }

    fn push_state(&self) {
        bridge::snapshot(&self.ui, self.model.snapshot());
    }
}
