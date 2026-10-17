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

/// Makes the kernel read the modes of every connected screen (EDID).
///
/// Slint only asks for the modes the kernel already knows. Usually the kernel's text console
/// has read them at boot, but without one (no /dev/fb0, e.g. MainsailOS on Debian trixie)
/// the list stays empty and Slint finds no usable mode.
pub fn probe() {
    use drm::control::Device;
    let Ok(entries) = std::fs::read_dir("/dev/dri") else { return };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("card") {
            continue;
        }
        let Ok(file) = OpenOptions::new().read(true).write(true).open(entry.path()) else { continue };
        let card = Card(file);
        // Render-only devices (e.g. the Raspberry Pi's v3d) have no connectors
        let Ok(resources) = card.resource_handles() else { continue };
        for connector in resources.connectors() {
            let _ = card.get_connector(*connector, true);
        }
    }
}
