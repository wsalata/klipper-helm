//! Passing data from the network thread to the UI (Slint main thread).

use crate::printer::{FileRow, Snapshot};
use crate::thumbs::{Pixels, Size};
use crate::updates::UpdateStatus;
use crate::{AppWindow, Conn, ConsoleLine, FileDetails, FileEntry, Messages, Printer, Status, Update};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel, Weak};
use std::cell::RefCell;
use std::collections::HashMap;

const CONSOLE_LIMIT: usize = 300;
/// Large thumbnails take ~0.4 MB each, so only the last few are kept
const LARGE_LIMIT: usize = 6;

/// Thumbnails by G-code file path. UI thread only (`Image` is not Send).
#[derive(Default)]
struct ThumbCache {
    small: HashMap<String, Image>,
    large: HashMap<String, Image>,
}

thread_local! {
    static THUMBS: RefCell<ThumbCache> = RefCell::default();
}

/// Best available thumbnail for a large view (the small one as a fallback).
fn large_thumb(path: &str) -> Image {
    THUMBS.with_borrow(|c| c.large.get(path).or_else(|| c.small.get(path)).cloned().unwrap_or_default())
}

fn small_thumb(path: &str) -> Image {
    THUMBS.with_borrow(|c| c.small.get(path).cloned().unwrap_or_default())
}

/// Updates a VecModel in place so the UI does not rebuild whole lists.
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
        p.set_status(s.status);
        p.set_status_message(s.status_message.into());
        sync(&p.get_top_heaters(), s.top_heaters);
        let filename = s.job.filename.clone();
        p.set_job(s.job);
        p.set_job_thumb(large_thumb(&filename));
        p.set_position(s.position);
        p.set_calibration(s.calibration);
        p.set_mesh(s.mesh.info);
        sync(&p.get_mesh_cells(), s.mesh.cells);
        sync(&p.get_mesh_profiles(), s.mesh.profiles);
        p.set_level(s.level);
        sync(&p.get_screws(), s.screws);
        sync(&p.get_pid_heaters(), s.pid_heaters);
        p.set_pid_heater(s.pid_heater.into());
        p.set_extruder_cal(s.extruder_cal);
        p.set_extruder_temp(s.extruder_temp);
        p.set_has_extruder(s.has_extruder);
        p.set_has_part_fan(s.part_fan.is_some());
        p.set_part_fan(s.part_fan.unwrap_or(0.0));
        sync(&p.get_heaters(), s.heaters);
        sync(&p.get_sensors(), s.sensors);
        sync(&p.get_fans(), s.fans);
    });
}

/// Connection state before Klipper data is available (connecting / disconnected).
pub fn conn(ui: &Weak<AppWindow>, conn: Conn, status: Status) {
    with_printer(ui, move |p| {
        p.set_conn(conn);
        p.set_status(status);
        p.set_status_message("".into());
        sync(&p.get_top_heaters(), Vec::new());
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
        p.set_load_macro(find_macro(&macros, &["LOAD_FILAMENT", "FILAMENT_LOAD"]).into());
        p.set_unload_macro(find_macro(&macros, &["UNLOAD_FILAMENT", "FILAMENT_UNLOAD"]).into());
        sync(&p.get_macros(), macros.into_iter().map(SharedString::from).collect());
    });
}

/// First macro matching one of the names (case-insensitive), "" when there is none.
fn find_macro(macros: &[String], names: &[&str]) -> String {
    names
        .iter()
        .find_map(|n| macros.iter().find(|m| m.eq_ignore_ascii_case(n)))
        .cloned()
        .unwrap_or_default()
}

pub fn files(ui: &Weak<AppWindow>, files: Vec<FileRow>) {
    with_printer(ui, move |p| {
        let entries = files
            .into_iter()
            .map(|f| FileEntry {
                thumb: small_thumb(&f.path),
                path: f.path.into(),
                size: f.size.into(),
                modified: f.modified.into(),
            })
            .collect();
        sync(&p.get_files(), entries);
    });
}

pub fn details(ui: &Weak<AppWindow>, details: FileDetails) {
    with_printer(ui, move |p| {
        p.set_details_thumb(large_thumb(&details.path));
        p.set_details(details);
    });
}

/// A downloaded thumbnail: caches it and swaps it in everywhere that file is shown.
pub fn thumbnail(ui: &Weak<AppWindow>, path: String, size: Size, pixels: Pixels) {
    with_printer(ui, move |p| {
        let image = Image::from_rgba8(pixels);
        THUMBS.with_borrow_mut(|c| match size {
            Size::Small => {
                c.small.insert(path.clone(), image.clone());
            }
            Size::Large => {
                if c.large.len() >= LARGE_LIMIT {
                    c.large.clear();
                }
                c.large.insert(path.clone(), image.clone());
            }
        });

        if size == Size::Small {
            let files = p.get_files();
            if let Some(i) = files.iter().position(|f| f.path == path.as_str()) {
                let mut row = files.row_data(i).unwrap_or_default();
                row.thumb = image;
                files.set_row_data(i, row);
            }
        }
        if p.get_details().path == path.as_str() {
            p.set_details_thumb(large_thumb(&path));
        }
        if p.get_job().filename == path.as_str() {
            p.set_job_thumb(large_thumb(&path));
        }
    });
}

/// Appends console lines (in chronological order); the newest are shown at the top.
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

/// Clears everything about the previous printer (after the Moonraker address changes).
pub fn reset(ui: &Weak<AppWindow>) {
    with_printer(ui, |p| {
        THUMBS.with_borrow_mut(|c| *c = ThumbCache::default());
        sync(&p.get_heaters(), Vec::new());
        sync(&p.get_sensors(), Vec::new());
        sync(&p.get_top_heaters(), Vec::new());
        sync(&p.get_fans(), Vec::new());
        sync(&p.get_files(), Vec::new());
        sync(&p.get_macros(), Vec::new());
        p.set_load_macro(Default::default());
        p.set_unload_macro(Default::default());
        sync(&p.get_console(), Vec::new());
        p.set_job(Default::default());
        p.set_position(Default::default());
        p.set_calibration(Default::default());
        p.set_mesh(Default::default());
        sync(&p.get_mesh_cells(), Vec::new());
        sync(&p.get_mesh_profiles(), Vec::new());
        p.set_level(Default::default());
        sync(&p.get_screws(), Vec::new());
        sync(&p.get_pid_heaters(), Vec::new());
        p.set_pid_heater("".into());
        p.set_extruder_cal(Default::default());
        p.set_details(Default::default());
        p.set_job_thumb(Default::default());
        p.set_details_thumb(Default::default());
        p.set_hostname("Klipper".into());
        p.set_klipper_version("".into());
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

/// Automatic temperature reports ("B:60.1 /60.0 T0:215.3 /215.0", "ok T:...") that Klipper sends
/// during PID_CALIBRATE, M109/M190 and M105; hidden like Mainsail does (`^(?:ok\s+)?(B|C|T\d*):`).
pub fn is_temperature_report(text: &str) -> bool {
    let text = match text.strip_prefix("ok") {
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
        _ => text,
    };
    let rest = match text.as_bytes().first() {
        Some(b'B' | b'C') => &text[1..],
        Some(b'T') => text[1..].trim_start_matches(|c: char| c.is_ascii_digit()),
        _ => return false,
    };
    rest.starts_with(':')
}

/// New update manager status: the list, and whether Moonraker is still working.
pub fn updates(ui: &Weak<AppWindow>, status: UpdateStatus) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let u = ui.global::<Update>();
        sync(&u.get_items(), status.items);
        u.set_loaded(true);
        u.set_busy(status.busy);
        if !status.busy {
            u.set_message("".into());
        }
    });
}

/// A refresh or update started (empty message), reported progress, or finished.
pub fn update_progress(ui: &Weak<AppWindow>, message: String, complete: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let u = ui.global::<Update>();
        u.set_error("".into());
        u.set_busy(!complete);
        u.set_message(if complete { "".into() } else { message.into() });
    });
}

pub fn update_error(ui: &Weak<AppWindow>, message: String) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let u = ui.global::<Update>();
        u.set_busy(false);
        u.set_message("".into());
        u.set_error(message.into());
    });
}

/// Shows a raw message (e.g. a Klipper error) as an error toast.
pub fn toast(ui: &Weak<AppWindow>, text: impl Into<SharedString>) {
    let text = text.into();
    with_printer(ui, move |p| {
        p.set_toast_error(true);
        p.set_toast(text);
    });
}

/// Translated "no connection to Moonraker" toast.
pub fn no_connection(ui: &Weak<AppWindow>) {
    let _ = ui.upgrade_in_event_loop(|ui| ui.global::<Messages>().invoke_no_connection());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_reports() {
        for t in ["B:23.2 /0.0 T0:223.8 /215.0", "ok T:21.0 /0.0 B:22.0 /0.0", "T:210.0 /210.0", "C:30.0"] {
            assert!(is_temperature_report(t), "{t}");
        }
        for t in ["// PID parameters: pid_Kp=22.2", "BED_MESH_CALIBRATE", "ok", "Toolhead: X:10", "!! Move out of range"] {
            assert!(!is_temperature_report(t), "{t}");
        }
    }
}
