//! Thumbnails through seq_manip's own `thumbs.rs` (EXR with the ACES
//! view, stills, movies via ffmpeg), decoded into egui images.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use egui::ColorImage;
use imfram::gallery::{Request, Thumbs};
use seq_core::thumbs::{thumbnail_data_url, ColorOpts};

/// Most frames a tile scrubs through (spread evenly over the sequence).
pub const SCRUB: usize = 48;

/// What the loader gets: an entity's frames. Equal by first frame only,
/// so hashing never walks a long frame list.
#[derive(Clone, Debug)]
pub struct Src {
    pub frames: Arc<Vec<PathBuf>>,
}

impl Src {
    fn key(&self) -> Option<&PathBuf> {
        self.frames.first()
    }
    /// How many scrub positions this source has.
    pub fn count(&self) -> u32 {
        self.frames.len().clamp(1, SCRUB) as u32
    }
    /// The file at scrub position `f`.
    pub fn at(&self, f: u32) -> Option<&Path> {
        let n = self.frames.len();
        if n == 0 {
            return None;
        }
        let i = (f as usize * n / self.count() as usize).min(n - 1);
        Some(&self.frames[i])
    }
}

impl PartialEq for Src {
    fn eq(&self, o: &Self) -> bool {
        self.key() == o.key()
    }
}
impl Eq for Src {}
impl Hash for Src {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.key().hash(h)
    }
}

pub fn cache_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Library/Caches/seq_manip_imfram/thumbs")
}

pub fn new() -> Thumbs<Src> {
    let cache = cache_dir();
    Thumbs::new(4, move |r: &Request<Src>| load(r, &cache))
}

fn load(r: &Request<Src>, cache: &Path) -> Result<ColorImage, String> {
    let path = r.src.at(r.frame).ok_or("no frames")?;
    let url = thumbnail_data_url(path, cache, &ColorOpts::default())?.ok_or("no picture")?;
    let b64 = url.split_once(',').map(|(_, b)| b).ok_or("not a data url")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let img = if img.width().max(img.height()) > r.max_px { img.thumbnail(r.max_px, r.max_px) } else { img };
    let rgba = img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}
