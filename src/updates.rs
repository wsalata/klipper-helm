//! Software updates through Moonraker's update manager, the same list Mainsail shows:
//! Klipper, Moonraker, the web interface, this panel (type "executable", set up by
//! install.sh) and the system packages.

use crate::UpdateItem;
use serde_json::Value;

/// Update manager status (`machine.update.status`, `notify_update_refreshed`).
pub struct UpdateStatus {
    pub busy: bool,
    pub items: Vec<UpdateItem>,
}

pub fn parse_status(status: &Value) -> UpdateStatus {
    let mut items: Vec<UpdateItem> = status
        .get("version_info")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, info)| item(name, info))
        .collect();
    // As in Mainsail: alphabetical, the operating system's packages last
    items.sort_by(|a, b| (a.name == "system", a.name.to_lowercase()).cmp(&(b.name == "system", b.name.to_lowercase())));
    UpdateStatus { busy: status.get("busy").and_then(Value::as_bool).unwrap_or(false), items }
}

fn item(name: &str, info: &Value) -> UpdateItem {
    let s = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let flag = |k: &str| info.get(k).and_then(Value::as_bool).unwrap_or(false);

    if name == "system" {
        let packages = info.get("package_count").and_then(Value::as_i64).unwrap_or(0);
        return UpdateItem {
            name: name.into(),
            state: if packages > 0 { "available" } else { "up-to-date" }.into(),
            packages: packages as i32,
            ..UpdateItem::default()
        };
    }

    let version = s("version");
    let remote = s("remote_version");
    let valid = info.get("is_valid").and_then(Value::as_bool).unwrap_or(true);
    let behind = info.get("commits_behind").and_then(Value::as_array).map_or(0, Vec::len);
    let state = if !valid || flag("corrupt") {
        "invalid"
    } else if behind > 0 || (!remote.is_empty() && remote != "?" && remote != version) {
        "available"
    } else {
        "up-to-date"
    };
    UpdateItem {
        name: name.into(),
        version: version.into(),
        remote: if remote == "?" { String::new() } else { remote }.into(),
        state: state.into(),
        packages: 0,
    }
}

/// Last non-empty line of an update's output (`notify_update_response`), for the status line.
pub fn last_line(message: &str) -> String {
    message.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status() {
        let status = parse_status(&json!({
            "busy": false,
            "version_info": {
                "system": { "name": "system", "package_count": 3, "package_list": [] },
                "moonraker": { "version": "v0.9.3-10", "remote_version": "v0.9.3-12", "is_valid": true,
                               "commits_behind": [{}, {}] },
                "Mainsail": { "version": "v2.14.0", "remote_version": "v2.14.0", "is_valid": true },
                "klipper-helm": { "version": "v0.3.0", "remote_version": "v0.3.1", "is_valid": true },
                "klipper": { "version": "v0.13.0", "remote_version": "?", "is_valid": false },
            },
        }));
        let rows: Vec<_> = status
            .items
            .iter()
            .map(|i| (i.name.to_string(), i.state.to_string(), i.remote.to_string(), i.packages))
            .collect();
        assert_eq!(
            rows,
            [
                ("klipper".into(), "invalid".into(), "".into(), 0),
                ("klipper-helm".into(), "available".into(), "v0.3.1".into(), 0),
                ("Mainsail".into(), "up-to-date".into(), "v2.14.0".into(), 0),
                ("moonraker".into(), "available".into(), "v0.9.3-12".into(), 0),
                ("system".into(), "available".into(), "".into(), 3),
            ]
        );
        assert_eq!(last_line("Downloading...\nExtracting\n\n"), "Extracting");
    }
}
