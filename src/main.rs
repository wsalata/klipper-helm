mod bridge;
mod config;
mod moonraker;
mod printer;

use moonraker::Command;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedSender};

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    let cfg = config::Config::load();
    let ui = AppWindow::new()?;

    let printer = ui.global::<Printer>();
    printer.set_moonraker_url(cfg.moonraker_url.as_str().into());
    printer.set_presets(ModelRc::new(VecModel::from(
        cfg.presets
            .iter()
            .map(|p| Preset { name: p.name.as_str().into(), extruder: p.extruder, bed: p.bed })
            .collect::<Vec<_>>(),
    )));
    // Listy aktualizowane w miejscu przez bridge::sync
    printer.set_heaters(ModelRc::new(VecModel::<Heater>::default()));
    printer.set_sensors(ModelRc::new(VecModel::<Heater>::default()));
    printer.set_fans(ModelRc::new(VecModel::<Fan>::default()));
    printer.set_files(ModelRc::new(VecModel::<FileEntry>::default()));
    printer.set_macros(ModelRc::new(VecModel::<SharedString>::default()));
    printer.set_console(ModelRc::new(VecModel::<ConsoleLine>::default()));

    let (tx, rx) = mpsc::unbounded_channel::<Command>();
    wire_actions(&ui, &tx);

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
        match key.as_str() {
            "BS" => {
                line.pop();
            }
            "CLR" => line.clear(),
            "ENTER" => {
                let cmd = line.trim();
                if !cmd.is_empty() {
                    let _ = tx.send(Command::Console(cmd.to_string()));
                }
                line.clear();
            }
            // Ignorujemy klawisze sterujące z fizycznej klawiatury
            k if k.chars().all(|c| !c.is_control()) => line.push_str(k),
            _ => {}
        }
        printer.set_console_input(line.into());
    });
}

/// Tylko w buildzie debug: przechodzi przez wszystkie ekrany, zapisuje każdy
/// jako `page-N.ppm` w podanym katalogu i zamyka aplikację.
#[cfg(debug_assertions)]
fn snapshot_pages(ui: &AppWindow, dir: String) -> Timer {
    const PAGES: i32 = 8;
    let weak = ui.as_weak();
    let mut step = 0;
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(1500), move || {
        let Some(ui) = weak.upgrade() else { return };
        // Pierwszy krok czeka na dane z Moonrakera
        if step > 0 {
            let page = step - 1;
            match ui.window().take_snapshot() {
                Ok(buf) => {
                    let mut ppm = format!("P6 {} {} 255\n", buf.width(), buf.height()).into_bytes();
                    ppm.extend(buf.as_slice().iter().flat_map(|p| [p.r, p.g, p.b]));
                    let path = format!("{dir}/page-{page}.ppm");
                    if let Err(e) = std::fs::write(&path, ppm) {
                        eprintln!("{path}: {e}");
                    }
                }
                Err(e) => eprintln!("Zrzut ekranu: {e}"),
            }
            if page + 1 == PAGES {
                let _ = slint::quit_event_loop();
                return;
            }
        }
        ui.global::<Nav>().set_page(step);
        step += 1;
    });
    timer
}
