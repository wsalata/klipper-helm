//! G-code file thumbnails. Moonraker extracts them from the file into a `.thumbs`
//! directory and lists their paths in the metadata; we fetch them over HTTP and decode them to pixels.

use anyhow::Context;
use serde_json::Value;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// How many small thumbnails (file list) are downloaded at the same time
const PARALLEL_SMALL: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    /// For the file list
    Small,
    /// For the file dialog and the dashboard during a print
    Large,
}

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// Outlives a single connection so a reconnect does not download everything again.
pub struct Thumbs {
    base: String,
    agent: ureq::Agent,
    small_slots: Arc<Semaphore>,
    /// (G-code file, size, modification time)
    requested: HashSet<(String, Size, String)>,
}

impl Thumbs {
    pub fn new(moonraker_url: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build();
        Self {
            base: http_base(moonraker_url),
            agent: ureq::Agent::new_with_config(config),
            small_slots: Arc::new(Semaphore::new(PARALLEL_SMALL)),
            requested: HashSet::new(),
        }
    }

    /// Whether the small thumbnail for this version of the file has already been checked.
    pub fn known(&self, path: &str, modified: &Value) -> bool {
        self.requested.contains(&(path.to_string(), Size::Small, stamp(modified)))
    }

    /// URL of the thumbnail to download; None when the file has none or the small one was already fetched.
    /// Large ones are fetched every time, because the UI only keeps a few of them.
    fn wanted(&mut self, path: &str, meta: &Value, size: Size) -> Option<String> {
        if size == Size::Small && !self.requested.insert((path.to_string(), size, stamp(&meta["modified"]))) {
            return None;
        }
        let thumbs = meta.get("thumbnails")?.as_array()?;
        let width = |t: &&Value| t.get("width").and_then(Value::as_u64).unwrap_or(0);
        let pick = match size {
            Size::Small => thumbs.iter().filter(|t| width(t) >= 32).min_by_key(width),
            Size::Large => thumbs.iter().filter(|t| width(t) <= 600).max_by_key(width),
        }
        .or_else(|| thumbs.iter().max_by_key(width))?;
        let rel = pick.get("relative_path")?.as_str()?;

        // relative_path is relative to the directory of the G-code file
        let full = match path.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{rel}"),
            None => rel.to_string(),
        };
        Some(format!("{}/server/files/gcodes/{}", self.base, encode_path(&full)))
    }

    /// Downloads a thumbnail in the background and passes the pixels to `done`.
    pub fn fetch(
        &mut self,
        path: &str,
        meta: &Value,
        size: Size,
        done: impl FnOnce(Pixels) + Send + 'static,
    ) {
        let Some(url) = self.wanted(path, meta, size) else { return };
        let agent = self.agent.clone();
        let slots = (size == Size::Small).then(|| self.small_slots.clone());
        tokio::spawn(async move {
            // Large thumbnails (open dialog, current print) do not wait in the queue
            let _permit = match slots {
                Some(s) => s.acquire_owned().await.ok(),
                None => None,
            };
            let result = tokio::task::spawn_blocking(move || download(&agent, &url).with_context(|| url)).await;
            match result {
                Ok(Ok(pixels)) => done(pixels),
                Ok(Err(e)) => eprintln!("Thumbnail: {e:#}"),
                Err(e) => eprintln!("Thumbnail: {e}"),
            }
        });
    }
}

fn download(agent: &ureq::Agent, url: &str) -> anyhow::Result<Pixels> {
    let bytes = agent
        .get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()?;
    decode_png(&bytes)
}

pub fn decode_png(bytes: &[u8]) -> anyhow::Result<Pixels> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palettes and 16-bit images are converted to 8-bit RGB(A)/grayscale
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().context("PNG too large")?];
    let info = reader.next_frame(&mut buf)?;
    let data = &buf[..info.buffer_size()];

    let mut out = Pixels::new(info.width, info.height);
    let px = out.make_mut_slice();
    let rgba = |r, g, b, a| Rgba8Pixel { r, g, b, a };
    match info.color_type {
        png::ColorType::Rgba => {
            for (p, c) in px.iter_mut().zip(data.chunks_exact(4)) {
                *p = rgba(c[0], c[1], c[2], c[3]);
            }
        }
        png::ColorType::Rgb => {
            for (p, c) in px.iter_mut().zip(data.chunks_exact(3)) {
                *p = rgba(c[0], c[1], c[2], 255);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for (p, c) in px.iter_mut().zip(data.chunks_exact(2)) {
                *p = rgba(c[0], c[0], c[0], c[1]);
            }
        }
        png::ColorType::Grayscale => {
            for (p, &v) in px.iter_mut().zip(data) {
                *p = rgba(v, v, v, 255);
            }
        }
        png::ColorType::Indexed => anyhow::bail!("unexpanded PNG palette"),
    }
    Ok(out)
}

/// ws://host:7125/websocket → http://host:7125
pub fn http_base(ws_url: &str) -> String {
    let url = if let Some(rest) = ws_url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = ws_url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        ws_url.to_string()
    };
    url.trim_end_matches('/').trim_end_matches("/websocket").to_string()
}

/// Encodes a path for a URL (spaces and non-ASCII characters in file names), keeping '/'.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn stamp(modified: &Value) -> String {
    modified.as_f64().map(|m| format!("{m:.0}")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base_url() {
        assert_eq!(http_base("ws://192.168.1.5:7125/websocket"), "http://192.168.1.5:7125");
        assert_eq!(http_base("wss://printer.local/websocket/"), "https://printer.local");
    }

    #[test]
    fn path_encoding() {
        assert_eq!(encode_path("dir/.thumbs/my file ż.png"), "dir/.thumbs/my%20file%20%C5%BC.png");
    }

    #[test]
    fn picks_size_and_dir() {
        let mut t = Thumbs::new("ws://h:7125/websocket");
        let meta = json!({
            "modified": 100.0,
            "thumbnails": [
                { "width": 32, "relative_path": ".thumbs/a-32x32.png" },
                { "width": 300, "relative_path": ".thumbs/a-300x300.png" },
            ]
        });
        assert_eq!(
            t.wanted("sub/a.gcode", &meta, Size::Small).unwrap(),
            "http://h:7125/server/files/gcodes/sub/.thumbs/a-32x32.png"
        );
        assert_eq!(
            t.wanted("a.gcode", &meta, Size::Large).unwrap(),
            "http://h:7125/server/files/gcodes/.thumbs/a-300x300.png"
        );
        // The small one is not fetched twice, the large one is
        assert!(t.wanted("sub/a.gcode", &meta, Size::Small).is_none());
        assert!(t.known("sub/a.gcode", &json!(100.0)));
        assert!(t.wanted("a.gcode", &meta, Size::Large).is_some());
        // A file without thumbnails is remembered too, so it is not queried over and over
        assert!(t.wanted("b.gcode", &json!({ "modified": 5.0 }), Size::Small).is_none());
        assert!(t.known("b.gcode", &json!(5.0)));
    }
}
