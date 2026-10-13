//! Settings screen: the configuration draft in the UI, validation, saving to the file
//! and applying it without restarting the panel.

use crate::config::{self, Config};
use crate::moonraker::Command;
use crate::{AppWindow, Messages, Nav, Preset, Printer, Settings, bridge};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc::UnboundedSender;

pub fn presets_model(presets: &[config::Preset]) -> Vec<Preset> {
    presets
        .iter()
        .map(|p| Preset { name: p.name.as_str().into(), extruder: p.extruder, bed: p.bed })
        .collect()
}

/// Switches the UI language live. "auto" follows LC_ALL / LC_MESSAGES / LANG;
/// a language without a translation falls back to English.
pub fn apply_language(language: &str) {
    let code = match language {
        "auto" | "" => system_language(),
        other => other.to_string(),
    };
    if slint::select_bundled_translation(&code).is_err() {
        let _ = slint::select_bundled_translation("");
    }
}

fn system_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|v| !v.is_empty())
        // "pl_PL.UTF-8" -> "pl"
        .and_then(|v| v.split(['_', '.', '@']).next().map(str::to_lowercase))
        .unwrap_or_default()
}

pub fn wire(ui: &AppWindow, cfg: Rc<RefCell<Config>>, tx: UnboundedSender<Command>) {
    let settings = ui.global::<Settings>();

    let weak = ui.as_weak();
    let current = cfg.clone();
    settings.on_load(move || {
        let Some(ui) = weak.upgrade() else { return };
        let s = ui.global::<Settings>();
        let cfg = current.borrow();
        s.set_url(cfg.moonraker_url.as_str().into());
        s.set_language(cfg.language.as_str().into());
        s.set_xy_speed(cfg.xy_speed);
        s.set_z_speed(cfg.z_speed);
        s.set_presets(ModelRc::new(VecModel::from(presets_model(&cfg.presets))));
        s.set_config_path(Config::path().map(|p| p.display().to_string()).unwrap_or_default().into());
        set_error(&s, "", "");
    });

    let weak = ui.as_weak();
    settings.on_edit_number(move |id, value| {
        let Some(ui) = weak.upgrade() else { return };
        let s = ui.global::<Settings>();
        match id.as_str() {
            "xy" => s.set_xy_speed(value),
            "z" => s.set_z_speed(value),
            other => edit_preset(&s, other, |p, field| match field {
                "ext" => p.extruder = value,
                "bed" => p.bed = value,
                _ => {}
            }),
        }
    });

    let weak = ui.as_weak();
    settings.on_edit_text(move |id, value| {
        let Some(ui) = weak.upgrade() else { return };
        let s = ui.global::<Settings>();
        let value = value.trim();
        match id.as_str() {
            "url" => s.set_url(value.into()),
            other => edit_preset(&s, other, |p, field| {
                if field == "name" {
                    p.name = value.into();
                }
            }),
        }
    });

    let weak = ui.as_weak();
    settings.on_preset_add(move |name| {
        let Some(ui) = weak.upgrade() else { return };
        with_presets(&ui.global::<Settings>(), |vm| {
            vm.push(Preset { name, extruder: 200.0, bed: 60.0 });
        });
    });

    let weak = ui.as_weak();
    settings.on_preset_remove(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        with_presets(&ui.global::<Settings>(), |vm| {
            if (index as usize) < vm.row_count() {
                vm.remove(index as usize);
            }
        });
    });

    let weak = ui.as_weak();
    settings.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        let s = ui.global::<Settings>();
        let new = Config {
            moonraker_url: s.get_url().trim().to_string(),
            language: s.get_language().to_string(),
            xy_speed: s.get_xy_speed(),
            z_speed: s.get_z_speed(),
            presets: s
                .get_presets()
                .iter()
                .map(|p| config::Preset { name: p.name.trim().to_string(), extruder: p.extruder, bed: p.bed })
                .collect(),
            // Edited on the Network screen
            network: cfg.borrow().network.clone(),
        };
        if let Err(e) = new.validate() {
            set_error(&s, e.code, &e.arg);
            return;
        }
        if let Err(e) = new.save() {
            set_error(&s, "save", &e.to_string());
            return;
        }

        // Apply: language, presets and address in the UI right away, the rest in the network thread
        apply_language(&new.language);
        let printer = ui.global::<Printer>();
        bridge::sync(&printer.get_presets(), presets_model(&new.presets));
        printer.set_moonraker_url(new.moonraker_url.as_str().into());
        let _ = tx.send(Command::Reconfigure(new.clone()));
        *cfg.borrow_mut() = new;

        set_error(&s, "", "");
        ui.global::<Nav>().set_page(7);
        ui.global::<Messages>().invoke_settings_saved();
    });
}

fn set_error(s: &Settings<'_>, code: &str, arg: &str) {
    s.set_error_code(code.into());
    s.set_error_arg(arg.into());
}

/// Adding and removing rows needs the concrete VecModel (set in `load`).
fn with_presets(s: &Settings<'_>, f: impl FnOnce(&VecModel<Preset>)) {
    let model = s.get_presets();
    if let Some(vm) = model.as_any().downcast_ref::<VecModel<Preset>>() {
        f(vm);
    }
}

/// id in the form "field:index", e.g. "ext:2"
fn edit_preset(s: &Settings<'_>, id: &str, change: impl FnOnce(&mut Preset, &str)) {
    let Some((field, index)) = id.split_once(':') else { return };
    let Ok(index) = index.parse::<usize>() else { return };
    let model = s.get_presets();
    if let Some(mut p) = model.row_data(index) {
        change(&mut p, field);
        model.set_row_data(index, p);
    }
}
