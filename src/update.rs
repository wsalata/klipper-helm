//! System updates: checks GitHub releases, downloads the signed RAUC bundle to the
//! data partition and installs it into the inactive A/B slot. RAUC verifies the
//! signature; U-Boot falls back to the current system if the new one does not boot.
//!
//! Installing requires KLIPPER_HELM_MANAGE_SYSTEM=1 (set by the embedded image's start
//! script); on a PC the panel can only check for new releases.

use crate::{AppWindow, Update};
use serde_json::Value;
use slint::{ComponentHandle, Weak};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const RELEASE_FILE: &str = "/etc/klipper-helm-release";
const DOWNLOAD_DIR: &str = "/data/update";
/// Bundles for this board are named klipper-helm-odroidxu4-<version>.raucb
const BUNDLE_PREFIX: &str = "klipper-helm-odroidxu4-";

enum UpdateCommand {
    Check,
    Install,
    Reboot,
}

/// The newest release that has a bundle for this board
#[derive(Clone)]
struct Release {
    version: String,
    notes: String,
    /// Public download link
    url: String,
    /// API link, which also works for private repositories with a token
    api_url: String,
    name: String,
    size: u64,
}

/// What the worker reports to the UI
enum Report {
    State(&'static str),
    /// No release with a bundle for this board (or the repository is private)
    NoRelease,
    Available { version: String, notes: String },
    Progress(f32),
    Error(String),
}

/// `token`: optional GitHub token for private repositories ("" for public ones)
pub fn wire(ui: &AppWindow, repo: String, token: String) {
    let managed = std::env::var("KLIPPER_HELM_MANAGE_SYSTEM").is_ok_and(|v| v == "1");
    let u = ui.global::<Update>();
    u.set_managed(managed);
    u.set_current_version(current_version().into());
    u.set_slot(booted_slot().into());
    u.set_state("idle".into());

    let (tx, rx) = mpsc::channel();
    let weak = ui.as_weak();
    std::thread::spawn(move || worker(weak, rx, repo, token, managed));

    let check = tx.clone();
    u.on_check(move || {
        let _ = check.send(UpdateCommand::Check);
    });
    let install = tx.clone();
    u.on_install(move || {
        let _ = install.send(UpdateCommand::Install);
    });
    u.on_reboot(move || {
        let _ = tx.send(UpdateCommand::Reboot);
    });
}

fn current_version() -> String {
    std::fs::read_to_string(RELEASE_FILE)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| format!("{}-dev", env!("CARGO_PKG_VERSION")))
}

/// "A" or "B" from the kernel command line (set by boot.cmd), "" when not on the image
fn booted_slot() -> String {
    std::fs::read_to_string("/proc/cmdline")
        .ok()
        .and_then(|c| c.split_whitespace().find_map(|w| w.strip_prefix("rauc.slot=")).map(str::to_string))
        .unwrap_or_default()
}

fn worker(ui: Weak<AppWindow>, rx: Receiver<UpdateCommand>, repo: String, token: String, managed: bool) {
    let report = |r: Report| {
        let _ = ui.upgrade_in_event_loop(move |ui| {
            let u = ui.global::<Update>();
            match r {
                Report::State(s) => u.set_state(s.into()),
                Report::NoRelease => u.set_state("no-release".into()),
                Report::Available { version, notes } => {
                    u.set_latest_version(version.into());
                    u.set_notes(notes.into());
                    u.set_state("available".into());
                }
                Report::Progress(p) => u.set_progress(p),
                Report::Error(e) => {
                    u.set_error(e.into());
                    u.set_state("error".into());
                }
            }
        });
    };

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .into();
    let mut latest: Option<Release> = None;

    for cmd in rx {
        match cmd {
            UpdateCommand::Check => {
                report(Report::State("checking"));
                match latest_release(&agent, &repo, &token) {
                    Ok(Some(r)) if is_newer(&r.version, &current_version()) => {
                        report(Report::Available { version: r.version.clone(), notes: r.notes.clone() });
                        latest = Some(r);
                    }
                    Ok(Some(_)) => report(Report::State("up-to-date")),
                    // GitHub answers 404 both when nothing is released and for a private
                    // repository without a token
                    Ok(None) | Err(CheckError::NotFound) => report(Report::NoRelease),
                    Err(CheckError::Other(e)) => report(Report::Error(e.to_string())),
                }
            }
            UpdateCommand::Install => {
                let Some(release) = latest.clone() else { continue };
                if !managed {
                    report(Report::Error("updates can only be installed on the panel's own system".into()));
                    continue;
                }
                report(Report::Progress(0.0));
                report(Report::State("downloading"));
                let bundle = match download(&agent, &release, &token, |p| report(Report::Progress(p))) {
                    Ok(path) => path,
                    Err(e) => {
                        report(Report::Error(format!("download failed: {e}")));
                        continue;
                    }
                };
                report(Report::Progress(0.0));
                report(Report::State("installing"));
                let result = install(&bundle, |p| report(Report::Progress(p)));
                let _ = std::fs::remove_file(&bundle);
                match result {
                    Ok(()) => report(Report::State("installed")),
                    Err(e) => report(Report::Error(format!("installation failed: {e}"))),
                }
            }
            UpdateCommand::Reboot => {
                if managed {
                    let _ = Command::new("reboot").status();
                }
            }
        }
    }
}

enum CheckError {
    NotFound,
    Other(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for CheckError {
    fn from(e: E) -> Self {
        CheckError::Other(e.into())
    }
}

fn get<'a>(agent: &'a ureq::Agent, url: &str, token: &str, accept: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
    let request = agent.get(url).header("Accept", accept);
    if token.is_empty() { request } else { request.header("Authorization", &format!("Bearer {token}")) }
}

fn latest_release(agent: &ureq::Agent, repo: &str, token: &str) -> Result<Option<Release>, CheckError> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut response = match get(agent, &url, token, "application/vnd.github+json").call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Err(CheckError::NotFound),
        Err(e) => return Err(e.into()),
    };
    let json: Value = serde_json::from_reader(response.body_mut().as_reader())?;
    let version = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
    let asset = json["assets"].as_array().into_iter().flatten().find(|a| {
        a["name"].as_str().is_some_and(|n| n.starts_with(BUNDLE_PREFIX) && n.ends_with(".raucb"))
    });
    let Some(asset) = asset else { return Ok(None) };
    Ok(Some(Release {
        version,
        notes: release_notes(json["body"].as_str().unwrap_or("")),
        url: asset["browser_download_url"].as_str().unwrap_or("").to_string(),
        api_url: asset["url"].as_str().unwrap_or("").to_string(),
        name: asset["name"].as_str().unwrap_or("update.raucb").to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
    }))
}

/// Release notes for the screen: Markdown markers removed, at most ~800 characters
fn release_notes(body: &str) -> String {
    let text: Vec<String> = body
        .lines()
        .map(|l| l.trim_start_matches('#').trim().replace("**", ""))
        .filter(|l| !l.is_empty())
        .collect();
    let mut notes = text.join("\n");
    if notes.chars().count() > 800 {
        notes = notes.chars().take(800).collect::<String>() + "...";
    }
    notes
}

/// Semantic version comparison; a pre-release ("0.2.0-dev") is older than its release
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v.split('+').next().unwrap_or(v), false),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    let (a, a_pre) = parse(candidate);
    let (b, b_pre) = parse(current);
    let len = a.len().max(b.len());
    let pad = |v: &[u64]| (0..len).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
    match pad(&a).cmp(&pad(&b)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => b_pre && !a_pre,
    }
}

fn download(agent: &ureq::Agent, release: &Release, token: &str, progress: impl Fn(f32)) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(DOWNLOAD_DIR)?;
    // Leftovers of an interrupted download
    for entry in std::fs::read_dir(DOWNLOAD_DIR)?.flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
    let path = Path::new(DOWNLOAD_DIR).join(&release.name);
    let partial = path.with_extension("part");
    // With a token, download through the API (works for private repositories too)
    let mut response = if token.is_empty() {
        agent.get(&release.url).call()?
    } else {
        get(agent, &release.api_url, token, "application/octet-stream").call()?
    };
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
        .unwrap_or(release.size)
        .max(1);
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
    let mut file = std::fs::File::create(&partial)?;
    let mut buf = vec![0u8; 64 * 1024];
    let (mut done, mut last) = (0u64, 0u64);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        // About every 1%, to keep the UI updates cheap
        if done - last > total / 100 {
            last = done;
            progress(done as f32 / total as f32);
        }
    }
    file.sync_all()?;
    std::fs::rename(&partial, &path)?;
    progress(1.0);
    Ok(path)
}

/// Runs `rauc install` and follows the percentages it prints
fn install(bundle: &Path, progress: impl Fn(f32)) -> anyhow::Result<()> {
    let mut child = Command::new("rauc")
        .arg("install")
        .arg(bundle)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(p) = percentage(&line) {
                progress(p);
            }
        }
    }
    let mut errors = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut errors);
    }
    let status = child.wait()?;
    if !status.success() {
        let last = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("rauc install failed");
        anyhow::bail!("{}", last.trim());
    }
    Ok(())
}

/// " 40% Copying image to rootfs.1" -> 0.4
fn percentage(line: &str) -> Option<f32> {
    let token = line.split_whitespace().find(|w| w.ends_with('%'))?;
    token.trim_end_matches('%').parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("0.1.0", "0.1.0-dev"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-rc1", "0.1.0"));
        assert!(!is_newer("0.9.9", "1.0.0"));
        assert!(is_newer("1.0.0", "0.10.0"));
    }

    #[test]
    fn reads_rauc_progress() {
        assert_eq!(percentage("  40% Copying image to rootfs.1"), Some(0.4));
        assert_eq!(percentage("100% Installing done."), Some(1.0));
        assert_eq!(percentage("installing"), None);
    }

    #[test]
    fn tidies_release_notes() {
        assert_eq!(release_notes("## What's new\n\n- **Wi-Fi** fixes\n"), "What's new\n- Wi-Fi fixes");
    }
}
