//! Choosing the screen and its mode for Slint's KMS backend.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd};

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl drm::Device for Card {}
impl drm::control::Device for Card {}

/// A display mode as offered by the kernel, in the kernel's (and so Slint's) order.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mode {
    width: u16,
    height: u16,
    refresh: u32,
    /// The screen's own choice (from its EDID), often wrong on cheap HDMI screens
    preferred: bool,
    /// Asked for on the kernel command line (`video=HDMI-A-1:1024x600@60`)
    user: bool,
}

/// Prepares the screen and returns the size of the mode the panel will use.
///
/// - Makes the kernel read the modes of every connected screen (EDID). Slint only asks for
///   the modes the kernel already knows; without the kernel's text console (no /dev/fb0,
///   e.g. MainsailOS on Debian trixie) nothing has read them and the list is empty.
/// - Picks the mode: SLINT_DRM_MODE if set, else KLIPPER_HELM_MODE ("1024x600" or
///   "1024x600@60"), else a mode from the kernel command line (`video=`), else the screen's
///   preferred mode as Slint would.
///
/// Must run first in main, before other threads exist (it sets environment variables).
pub fn setup() -> Option<(u16, u16)> {
    use drm::control::Device;
    let wanted_output = std::env::var("SLINT_DRM_OUTPUT").ok();
    let mut cards: Vec<_> = std::fs::read_dir("/dev/dri").ok()?.flatten().collect();
    // Same order as Slint (directory order), so both settle on the same screen
    cards.retain(|e| e.file_name().to_string_lossy().starts_with("card"));

    for entry in cards {
        let Ok(file) = OpenOptions::new().read(true).write(true).open(entry.path()) else { continue };
        let card = Card(file);
        // Render-only devices (e.g. the Raspberry Pi's v3d) have no connectors
        let Ok(resources) = card.resource_handles() else { continue };
        for handle in resources.connectors() {
            let Ok(connector) = card.get_connector(*handle, true) else { continue };
            let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
            let connected = connector.state() == drm::control::connector::State::Connected;
            if !connected || wanted_output.as_ref().is_some_and(|w| *w != name) {
                continue;
            }
            let modes: Vec<Mode> = connector
                .modes()
                .iter()
                .map(|m| {
                    let (width, height) = m.size();
                    let flags = m.mode_type();
                    Mode {
                        width,
                        height,
                        refresh: m.vrefresh(),
                        preferred: flags.contains(drm::control::ModeTypeFlags::PREFERRED),
                        user: flags.contains(drm::control::ModeTypeFlags::USERDEF),
                    }
                })
                .collect();
            return choose(&name, &modes);
        }
    }
    None
}

fn choose(output: &str, modes: &[Mode]) -> Option<(u16, u16)> {
    // An index chosen by hand (SLINT_DRM_MODE=list shows them) wins
    if let Some(index) = std::env::var("SLINT_DRM_MODE").ok().and_then(|v| v.parse::<usize>().ok()) {
        return modes.get(index).map(|m| (m.width, m.height));
    }
    let request = std::env::var("KLIPPER_HELM_MODE").ok().filter(|r| !r.trim().is_empty());
    let picked = match &request {
        Some(r) => {
            let found = find_mode(modes, r);
            if found.is_none() {
                let list: Vec<String> =
                    modes.iter().map(|m| format!("{}x{}@{}", m.width, m.height, m.refresh)).collect();
                eprintln!("KLIPPER_HELM_MODE={r}: no such mode on {output}; available: {}", list.join(", "));
            }
            found
        }
        None => modes.iter().position(|m| m.user),
    };
    let index = match picked {
        Some(index) => {
            // SAFETY: called first in main, before any other thread exists
            unsafe {
                std::env::set_var("SLINT_DRM_OUTPUT", output);
                std::env::set_var("SLINT_DRM_MODE", index.to_string());
            }
            index
        }
        None => slint_default(modes)?,
    };
    let m = modes[index];
    eprintln!("Display {output}: {}x{}@{}", m.width, m.height, m.refresh);
    Some((m.width, m.height))
}

/// "1024x600" or "1024x600@60": that size, the refresh rate closest to the one asked for (or 60).
fn find_mode(modes: &[Mode], request: &str) -> Option<usize> {
    let (size, refresh) = request.trim().split_once('@').unwrap_or((request.trim(), "60"));
    let (w, h) = size.split_once('x')?;
    let (w, h): (u16, u16) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    let refresh: u32 = refresh.trim().parse().unwrap_or(60);
    modes
        .iter()
        .enumerate()
        .filter(|(_, m)| m.width == w && m.height == h)
        .min_by_key(|(_, m)| m.refresh.abs_diff(refresh))
        .map(|(i, _)| i)
}

/// The mode Slint takes on its own: preferred first, then the largest.
fn slint_default(modes: &[Mode]) -> Option<usize> {
    modes
        .iter()
        .enumerate()
        .max_by_key(|(_, m)| (m.preferred, u32::from(m.width) * u32::from(m.height)))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u16, height: u16, refresh: u32, preferred: bool, user: bool) -> Mode {
        Mode { width, height, refresh, preferred, user }
    }

    #[test]
    fn picks_modes() {
        let modes = [
            mode(1024, 768, 60, true, false),
            mode(1024, 600, 50, false, false),
            mode(1024, 600, 60, false, false),
            mode(800, 600, 60, false, false),
        ];
        assert_eq!(find_mode(&modes, "1024x600"), Some(2));
        assert_eq!(find_mode(&modes, " 1024x600@50 "), Some(1));
        assert_eq!(find_mode(&modes, "640x480"), None);
        assert_eq!(find_mode(&modes, "nonsense"), None);
        assert_eq!(slint_default(&modes), Some(0));
        assert_eq!(slint_default(&[mode(800, 480, 60, false, false), mode(1280, 720, 60, false, false)]), Some(1));
    }
}
