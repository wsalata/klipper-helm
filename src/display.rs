//! Preparing the screen for Slint's KMS backend.

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
    preferred: bool,
}

/// Prepares the screen and returns the size of the mode Slint will use (for the UI scale).
///
/// Makes the kernel read the modes of every connected screen (EDID): Slint only asks for the
/// modes the kernel already knows, and without the kernel's text console (no /dev/fb0) nothing
/// has read them, so the list would be empty. The mode itself is left to Slint and the system
/// (SLINT_DRM_OUTPUT, SLINT_DRM_MODE, or the screen's preferred mode).
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
                    let preferred = m.mode_type().contains(drm::control::ModeTypeFlags::PREFERRED);
                    Mode { width, height, refresh: m.vrefresh(), preferred }
                })
                .collect();
            let Some(m) = slint_mode(&modes) else {
                eprintln!("Display {name}: the kernel offers no usable mode for this screen");
                return None;
            };
            eprintln!("Display {name}: {}x{}@{}", m.width, m.height, m.refresh);
            return Some((m.width, m.height));
        }
    }
    None
}

/// The mode Slint uses: the index in SLINT_DRM_MODE, else preferred first, then the largest.
fn slint_mode(modes: &[Mode]) -> Option<Mode> {
    if let Some(index) = std::env::var("SLINT_DRM_MODE").ok().and_then(|v| v.parse::<usize>().ok()) {
        return modes.get(index).copied();
    }
    preferred_mode(modes)
}

fn preferred_mode(modes: &[Mode]) -> Option<Mode> {
    modes.iter().copied().max_by_key(|m| (m.preferred, u32::from(m.width) * u32::from(m.height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u16, height: u16, preferred: bool) -> Mode {
        Mode { width, height, refresh: 60, preferred }
    }

    #[test]
    fn picks_modes_like_slint() {
        let modes = [mode(1024, 600, false), mode(1024, 768, true), mode(800, 600, false)];
        assert_eq!(preferred_mode(&modes), Some(modes[1]));
        assert_eq!(preferred_mode(&[mode(800, 480, false), mode(1280, 720, false)]), Some(mode(1280, 720, false)));
        assert_eq!(preferred_mode(&[]), None);
    }
}
