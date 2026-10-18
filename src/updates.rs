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
    // As in Mainsail: git repositories by the commits they are behind, the rest only when the
    // remote version is newer (Moonraker may still hold an older one from its last check)
    let available = if s("configured_type") == "git_repo" {
        info.get("commits_behind").and_then(Value::as_array).is_some_and(|c| !c.is_empty())
    } else {
        is_newer(&remote, &version)
    };
    let state = if !valid || flag("corrupt") {
        "invalid"
    } else if available {
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

/// "v1.2.3", "1.2.3-beta.1" or "v0.13.0-120" (a git describe: 120 commits after the tag).
/// Compared by the numbers; on equal numbers a release is newer than a pre-release, and a
/// later commit count newer than an earlier one. Unparsable versions: newer when different.
fn is_newer(remote: &str, local: &str) -> bool {
    fn parse(v: &str) -> Option<(Vec<u64>, Option<String>)> {
        let v = v.trim().trim_start_matches('v');
        let (core, suffix) = match v.split_once('-') {
            Some((c, s)) => (c, Some(s.to_string())),
            None => (v, None),
        };
        let numbers = core.split('.').map(|n| n.parse().ok()).collect::<Option<Vec<u64>>>()?;
        Some((numbers, suffix))
    }
    if remote.is_empty() || remote == "?" || remote == local {
        return false;
    }
    let (Some((r, rs)), Some((l, ls))) = (parse(remote), parse(local)) else { return true };
    if r != l {
        return r > l;
    }
    // Commits after the tag ("-120"), else a pre-release ("-beta.1") below the release
    let count = |s: &Option<String>| s.as_deref().and_then(|s| s.split('-').next()?.parse::<u64>().ok());
    match (&rs, &ls) {
        (None, None) => false,
        (None, Some(_)) => count(&ls).is_none(),
        (Some(_), None) => count(&rs).is_some(),
        (Some(a), Some(b)) => match (count(&rs), count(&ls)) {
            (Some(x), Some(y)) => x > y,
            _ => a > b,
        },
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

    #[test]
    fn versions() {
        assert!(is_newer("v0.4.0", "v0.3.2"));
        assert!(!is_newer("v0.3.2", "v0.4.0"));
        assert!(!is_newer("v0.4.0", "v0.4.0"));
        assert!(is_newer("v0.10.0", "v0.9.9"));
        assert!(is_newer("v0.13.0-131", "v0.13.0-120"));
        assert!(!is_newer("v0.13.0-120", "v0.13.0-131"));
        assert!(is_newer("v1.0.0", "v1.0.0-beta.2"));
        assert!(!is_newer("v1.0.0-beta.2", "v1.0.0"));
        assert!(is_newer("v1.0.0-beta.2", "v1.0.0-beta.1"));
        assert!(!is_newer("", "v1.0.0"));
        assert!(!is_newer("?", "v1.0.0"));
        assert!(is_newer("nightly-2", "nightly-1"));
    }

    #[test]
    fn older_remote_is_not_an_update() {
        let status = parse_status(&json!({ "version_info": {
            "klipper-helm": { "configured_type": "executable", "version": "v0.4.0",
                              "remote_version": "v0.3.2", "is_valid": true },
            "klipper": { "configured_type": "git_repo", "version": "v0.13.0-120",
                         "remote_version": "v0.13.0-131", "is_valid": true, "commits_behind": [] },
        }}));
        assert!(status.items.iter().all(|i| i.state == "up-to-date"));
    }
}
