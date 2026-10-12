//! Przekazywanie danych z wątku sieciowego do UI (wątek główny Slinta).

use crate::printer::Snapshot;
use crate::{AppWindow, Conn, ConsoleLine, FileDetails, FileEntry, Printer};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

const CONSOLE_LIMIT: usize = 300;

/// Aktualizuje VecModel w miejscu, żeby nie przebudowywać całych list w UI.
pub fn sync<T: Clone + PartialEq + 'static>(model: &ModelRc<T>, items: Vec<T>) {
    let Some(vm) = model.as_any().downcast_ref::<VecModel<T>>() else { return };
    if vm.row_count() == items.len() {
        for (i, item) in items.into_iter().enumerate() {
            if vm.row_data(i).as_ref() != Some(&item) {
                vm.set_row_data(i, item);
            }
        }
    } else {
        vm.set_vec(items);
    }
}

fn with_printer(ui: &Weak<AppWindow>, f: impl FnOnce(Printer<'_>) + Send + 'static) {
    let _ = ui.upgrade_in_event_loop(move |ui| f(ui.global::<Printer>()));
}

pub fn snapshot(ui: &Weak<AppWindow>, s: Snapshot) {
    with_printer(ui, move |p| {
        p.set_conn(s.conn);
        p.set_status_label(s.status_label.into());
        p.set_status_message(s.status_message.into());
        p.set_top_temps(s.top_temps.into());
        p.set_job(s.job);
        p.set_position(s.position);
        p.set_extruder_temp(s.extruder_temp);
        p.set_has_extruder(s.has_extruder);
        sync(&p.get_heaters(), s.heaters);
        sync(&p.get_sensors(), s.sensors);
        sync(&p.get_fans(), s.fans);
    });
}

pub fn conn(ui: &Weak<AppWindow>, conn: Conn, label: &str, message: String) {
    let label = SharedString::from(label);
    with_printer(ui, move |p| {
        p.set_conn(conn);
        p.set_status_label(label);
        p.set_status_message(message.into());
        p.set_top_temps("".into());
    });
}

pub fn info(ui: &Weak<AppWindow>, hostname: String, version: String) {
    with_printer(ui, move |p| {
        if !hostname.is_empty() {
            p.set_hostname(hostname.into());
        }
        p.set_klipper_version(version.into());
    });
}

pub fn macros(ui: &Weak<AppWindow>, macros: Vec<String>) {
    with_printer(ui, move |p| {
        sync(&p.get_macros(), macros.into_iter().map(SharedString::from).collect());
    });
}

pub fn files(ui: &Weak<AppWindow>, files: Vec<FileEntry>) {
    with_printer(ui, move |p| sync(&p.get_files(), files));
}

pub fn details(ui: &Weak<AppWindow>, details: FileDetails) {
    with_printer(ui, move |p| p.set_details(details));
}

/// Dodaje linie konsoli (w kolejności chronologicznej); najnowsze są wyświetlane na górze.
pub fn console(ui: &Weak<AppWindow>, lines: Vec<ConsoleLine>) {
    with_printer(ui, move |p| {
        let model = p.get_console();
        let Some(vm) = model.as_any().downcast_ref::<VecModel<ConsoleLine>>() else { return };
        for line in lines {
            vm.insert(0, line);
        }
        while vm.row_count() > CONSOLE_LIMIT {
            vm.remove(vm.row_count() - 1);
        }
    });
}

pub fn console_line(text: &str) -> ConsoleLine {
    let kind = if text.starts_with("!!") {
        2
    } else if text.starts_with("//") {
        3
    } else {
        0
    };
    ConsoleLine { text: text.into(), kind }
}

pub fn toast(ui: &Weak<AppWindow>, text: impl Into<SharedString>) {
    let text = text.into();
    with_printer(ui, move |p| p.set_toast(text));
}
