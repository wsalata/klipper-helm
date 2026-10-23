//! Editing printer.cfg through Moonraker's file API, for values that Klipper's SAVE_CONFIG
//! does not keep (the extruder's rotation_distance).

use anyhow::{Context, bail};
use std::time::Duration;

const FILE: &str = "printer.cfg";
const BOUNDARY: &str = "klipper-helm-7f3a9c2e51d84b06";

/// Sets `option` in `[section]` of printer.cfg on the printer (`base`: Moonraker's HTTP address).
/// Ok(false) when printer.cfg does not hold that option exactly once (e.g. the section lives in an
/// included file); the file is not touched then.
pub fn save_option(base: &str, section: &str, option: &str, value: &str) -> anyhow::Result<bool> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let url = format!("{base}/server/files/config/{FILE}");
    let text = agent.get(&url).call().with_context(|| url.clone())?.body_mut().read_to_string()?;
    let Some(text) = set_option(&text, section, option, value) else { return Ok(false) };
    if text.contains(BOUNDARY) {
        bail!("{FILE} contains the upload boundary");
    }
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"root\"\r\n\r\nconfig\r\n\
         --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{FILE}\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n{text}\r\n--{BOUNDARY}--\r\n"
    );
    let url = format!("{base}/server/files/upload");
    agent
        .post(&url)
        .header("Content-Type", format!("multipart/form-data; boundary={BOUNDARY}"))
        .send(body)
        .with_context(|| url.clone())?;
    Ok(true)
}

/// The config text with `option` in `[section]` set to `value`, keeping the spacing and any
/// comment on its line. None unless the option appears there exactly once. SAVE_CONFIG's
/// block at the end (`#*#` lines) belongs to Klipper and is left alone.
pub fn set_option(text: &str, section: &str, option: &str, value: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() + value.len());
    let mut current = "";
    let mut found = 0;
    let mut autosave = false;
    for line in text.split_inclusive('\n') {
        autosave |= line.starts_with("#*#");
        if !autosave {
            if let Some((name, _)) = line.strip_prefix('[').and_then(|r| r.split_once(']')) {
                current = name.trim();
            } else if current == section
                && let Some(new) = replace_value(line, option, value)
            {
                found += 1;
                out.push_str(&new);
                continue;
            }
        }
        out.push_str(line);
    }
    (found == 1).then_some(out)
}

/// "option: old  # comment\n" -> "option: value  # comment\n". Indented lines continue the
/// previous option's value, so only lines starting with the option name match.
fn replace_value(line: &str, option: &str, value: &str) -> Option<String> {
    let rest = line.strip_prefix(option)?;
    let sep = rest.trim_start();
    // Not a longer name such as rotation_distance_x
    if !sep.starts_with([':', '=']) {
        return None;
    }
    let start = line.len() - sep.len() + 1;
    let after = &line[start..];
    let end = after.find(['#', ';']).unwrap_or_else(|| after.trim_end_matches(['\r', '\n']).len());
    let (old, tail) = after.split_at(end);
    // An empty value continues on the next lines; not something to rewrite
    if old.trim().is_empty() {
        return None;
    }
    let lead = &old[..old.len() - old.trim_start().len()];
    let trail = &old[old.trim_end().len()..];
    Some(format!("{}{lead}{value}{trail}{tail}", &line[..start]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "[include mainsail.cfg]\n\
        [extruder_stepper belt]\nrotation_distance: 40\n\
        [extruder]\nstep_pin: PB3\nrotation_distance_x: 1\nrotation_distance : 22.6789  # 5mm gear\r\nnozzle_diameter: 0.4\n\
        [stepper_x]\nrotation_distance: 40\n\
        #*# <---------------------- SAVE_CONFIG ---------------------->\n#*# [extruder]\n#*# rotation_distance: 1\n";

    #[test]
    fn sets_only_the_section_option() {
        let new = set_option(CFG, "extruder", "rotation_distance", "23.100").unwrap();
        assert_eq!(new, CFG.replace("rotation_distance : 22.6789  #", "rotation_distance : 23.100  #"));
    }

    #[test]
    fn keeps_spacing_without_comment() {
        let cfg = "[extruder]\nrotation_distance=7.5\n";
        assert_eq!(set_option(cfg, "extruder", "rotation_distance", "7.71").unwrap(), "[extruder]\nrotation_distance=7.71\n");
        let cfg = "[extruder]\nrotation_distance: 7.5";
        assert_eq!(set_option(cfg, "extruder", "rotation_distance", "8").unwrap(), "[extruder]\nrotation_distance: 8");
    }

    #[test]
    fn missing_or_ambiguous() {
        assert_eq!(set_option("[include extruder.cfg]\n[extruder]\nstep_pin: PB3\n", "extruder", "rotation_distance", "1"), None);
        let twice = "[extruder]\nrotation_distance: 1\n[extruder]\nrotation_distance: 2\n";
        assert_eq!(set_option(twice, "extruder", "rotation_distance", "3"), None);
        // Only in SAVE_CONFIG's block
        assert_eq!(set_option("#*# [extruder]\n#*# rotation_distance: 1\n", "extruder", "rotation_distance", "3"), None);
    }
}
