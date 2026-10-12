//! Passing data from the network thread to the UI (Slint main thread).

use crate::printer::{FileRow, Snapshot};
use crate::thumbs::{Pixels, Size};
use crate::{AppWindow, Conn, ConsoleLine, FileDetails, FileEntry, Messages, Printer, Status};
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
        p.set_extruder_temp(s.extruder_temp);
        p.set_has_extruder(s.has_extruder);
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
        sync(&p.get_macros(), macros.into_iter().map(SharedString::from).collect());
    });
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
        sync(&p.get_console(), Vec::new());
        p.set_job(Default::default());
        p.set_position(Default::default());
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
