mod bridge;
mod config;
mod moonraker;
mod printer;
mod settings;
mod thumbs;

use moonraker::Command;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedSender};

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    let cfg = config::Config::load();
    let ui = AppWindow::new()?;

    let printer = ui.global::<Printer>();
    printer.set_moonraker_url(cfg.moonraker_url.as_str().into());
    printer.set_presets(ModelRc::new(VecModel::from(settings::presets_model(&cfg.presets))));
    // Listy aktualizowane w miejscu przez bridge::sync
    printer.set_heaters(ModelRc::new(VecModel::<Heater>::default()));
    printer.set_sensors(ModelRc::new(VecModel::<Heater>::default()));
    printer.set_fans(ModelRc::new(VecModel::<Fan>::default()));
    printer.set_files(ModelRc::new(VecModel::<FileEntry>::default()));
    printer.set_macros(ModelRc::new(VecModel::<SharedString>::default()));
    printer.set_console(ModelRc::new(VecModel::<ConsoleLine>::default()));

    let (tx, rx) = mpsc::unbounded_channel::<Command>();
    wire_actions(&ui, &tx);
    settings::wire(&ui, Rc::new(RefCell::new(cfg.clone())), tx.clone());

    let clock = Timer::default();
    let weak = ui.as_weak();
    let tick = move || {
        if let Some(ui) = weak.upgrade() {
            ui.global::<Printer>().set_clock(chrono::Local::now().format("%H:%M").to_string().into());
        }
    };
    tick();
    clock.start(TimerMode::Repeated, Duration::from_secs(1), tick);

    // Sieć działa w osobnym wątku z tokio, UI (Slint) musi zostać w wątku głównym
    let weak = ui.as_weak();
    let net = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(moonraker::run(cfg, weak, rx));
    });

    #[cfg(debug_assertions)]
    let _snapshots = std::env::var("KLIPPER_HELM_SNAPSHOT").ok().map(|dir| snapshot_pages(&ui, dir));

    ui.run()?;
    // Zamknięcie kanału kończy wątek sieciowy
    drop(ui);
    drop(tx);
    let _ = net.join();
    Ok(())
}

fn wire_actions(ui: &AppWindow, tx: &UnboundedSender<Command>) {
    let actions = ui.global::<Actions>();

    macro_rules! on {
        ($setter:ident, || $cmd:expr) => {
            on!($setter, | | $cmd)
        };
        ($setter:ident, |$($arg:ident),*| $cmd:expr) => {{
            let tx = tx.clone();
            actions.$setter(move |$($arg),*| {
                let _ = tx.send($cmd);
            });
        }};
    }

    on!(on_gcode, |g| Command::Gcode(g.into()));
    on!(on_set_temp, |h, t| Command::SetTemp(h.into(), t));
    on!(on_set_fan, |f, s| Command::SetFan(f.into(), s));
    on!(on_jog, |axis, dist| Command::Jog(axis.into(), dist));
    on!(on_extrude, |mm, speed| Command::Extrude(mm, speed));
    on!(on_pause, || Command::Pause);
    on!(on_resume, || Command::Resume);
    on!(on_cancel, || Command::Cancel);
    on!(on_emergency_stop, || Command::EmergencyStop);
    on!(on_restart, || Command::Restart);
    on!(on_firmware_restart, || Command::FirmwareRestart);
    on!(on_refresh_files, || Command::RefreshFiles);
    on!(on_file_details, |f| Command::FileDetails(f.into()));
    on!(on_start_print, |f| Command::StartPrint(f.into()));
    on!(on_host_reboot, || Command::HostReboot);
    on!(on_host_shutdown, || Command::HostShutdown);

    // Edycja linii konsoli odbywa się lokalnie, do sieci idzie dopiero ENTER
    let weak = ui.as_weak();
    let tx = tx.clone();
    actions.on_console_key(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        let printer = ui.global::<Printer>();
        let mut line = printer.get_console_input().to_string();
        if key == "ENTER" {
            let cmd = line.trim();
            if !cmd.is_empty() {
                let _ = tx.send(Command::Console(cmd.to_string()));
            }
            line.clear();
        } else {
            edit_line(&mut line, &key);
        }
        printer.set_console_input(line.into());
    });

    // Okno edycji tekstu (ustawienia)
    let weak = ui.as_weak();
    actions.on_text_key(move |key| {
        let Some(ui) = weak.upgrade() else { return };
        let overlay = ui.global::<Overlay>();
        let mut line = overlay.get_text_value().to_string();
        edit_line(&mut line, &key);
        overlay.set_text_value(line.into());
    });
}

/// Wspólna obsługa klawiszy edycji: BS, CLR albo znaki do dopisania.
fn edit_line(line: &mut String, key: &str) {
    match key {
        "BS" => {
            line.pop();
        }
        "CLR" => line.clear(),
        // Ignorujemy klawisze sterujące z fizycznej klawiatury
        k if k.chars().all(|c| !c.is_control()) => line.push_str(k),
        _ => {}
    }
}

/// Tylko w buildzie debug: przechodzi przez kolejne widoki (ekrany, okna),
/// zapisuje każdy jako `page-N.ppm` w podanym katalogu i zamyka aplikację.
#[cfg(debug_assertions)]
fn snapshot_pages(ui: &AppWindow, dir: String) -> Timer {
    type Step = Box<dyn Fn(&AppWindow)>;
    let page = |n: i32| -> Step { Box::new(move |ui| ui.global::<Nav>().set_page(n)) };
    let mut steps: Vec<Step> = (0..8).map(page).collect();
    steps.push(Box::new(|ui| {
        if let Some(file) = ui.global::<Printer>().get_files().row_data(0) {
            ui.global::<Nav>().set_page(4);
            ui.global::<Overlay>().invoke_open_file(file.path.clone());
            ui.global::<Actions>().invoke_file_details(file.path);
        }
    }));
    steps.push(Box::new(|ui| {
        ui.global::<Overlay>().set_file_open(false);
        ui.global::<Settings>().invoke_load();
        ui.global::<Nav>().set_page(8);
    }));
    steps.push(Box::new(|ui| {
        let url = ui.global::<Settings>().get_url();
        ui.global::<Overlay>().invoke_ask_text("url".into(), "Adres Moonrakera".into(), url);
    }));
    steps.push(Box::new(|ui| {
        ui.global::<Overlay>().set_text_open(false);
        ui.global::<Overlay>().invoke_ask_number("xy".into(), "Prędkość XY [mm/s]".into(), 100.0, " mm/s".into());
    }));

    let weak = ui.as_weak();
    let mut step = 0;
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(1500), move || {
        let Some(ui) = weak.upgrade() else { return };
        // Pierwszy krok czeka na dane z Moonrakera
        if step > 0 {
            let n = step - 1;
            match ui.window().take_snapshot() {
                Ok(buf) => {
                    let mut ppm = format!("P6 {} {} 255\n", buf.width(), buf.height()).into_bytes();
                    ppm.extend(buf.as_slice().iter().flat_map(|p| [p.r, p.g, p.b]));
                    let path = format!("{dir}/page-{n}.ppm");
                    if let Err(e) = std::fs::write(&path, ppm) {
                        eprintln!("{path}: {e}");
                    }
                }
                Err(e) => eprintln!("Zrzut ekranu: {e}"),
            }
        }
        match steps.get(step) {
            Some(setup) => setup(&ui),
            None => {
                let _ = slint::quit_event_loop();
                return;
            }
        }
        step += 1;
    });
    timer
}
