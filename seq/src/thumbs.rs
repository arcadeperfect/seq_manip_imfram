//! Pictures for tiles and rows through the app's own commands, the way the
//! Svelte app gets them: a sequence's frames from `playback_frame` (every
//! frame reachable by scrubbing, sized to the tile), a movie's from
//! `scrub_thumbnails`, a still's from `thumbnail` / `playback_frame`. All
//! land in the Tauri app's own disk cache, so the two apps share it.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use egui::ColorImage;
use imfram::gallery::{Request, Thumbs};
use seq_core::backend::{Backend, Color};
use seq_core::state::EntityId;

use crate::model::{Kind, Row};

/// Scrub positions across a movie.
pub const MOVIE_STEPS: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum What {
    Sequence,
    Movie,
    Still,
}

/// What the loader gets. Equal while the files behind it are the same.
#[derive(Clone, Debug)]
pub struct Src {
    pub id: EntityId,
    pub what: What,
    pub frames: u32,
    key: u64,
    color: Color,
}

impl PartialEq for Src {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id && self.key == o.key && self.color == o.color
    }
}
impl Eq for Src {}
impl Hash for Src {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.id.hash(h);
        self.key.hash(h);
    }
}

impl Src {
    pub fn of(r: &Row, color: Color, displayable: bool) -> Option<Self> {
        if !displayable {
            return None;
        }
        let (what, frames) = match r.kind {
            Kind::Sequence => (What::Sequence, r.frames().max(1) as u32),
            Kind::File if r.is_movie() => (What::Movie, MOVIE_STEPS),
            Kind::File => (What::Still, 1),
            _ => return None,
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (r.dir.as_os_str(), r.canonical_name(), frames).hash(&mut h);
        Some(Src { id: r.id, what, frames, key: h.finish(), color })
    }
    /// The frame shown at rest: a sequence's middle (as the Svelte app), a movie's second step.
    pub fn poster(&self) -> u32 {
        match self.what {
            What::Sequence => self.frames / 2,
            What::Movie => 1,
            What::Still => 0,
        }
    }
}

pub fn new(backend: Arc<Backend>) -> Thumbs<Src> {
    // A movie's steps arrive together; keep them for the frames that follow.
    let movies: Arc<Mutex<HashMap<(EntityId, u64), Arc<Vec<Vec<u8>>>>>> = Arc::default();
    Thumbs::new(8, move |r: &Request<Src>| load(&backend, &movies, r))
}

fn decode(bytes: &[u8], max_px: u32) -> Result<ColorImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if img.width().max(img.height()) > max_px { img.thumbnail(max_px, max_px) } else { img };
    let rgba = img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// The size asked of `playback_frame`: the tile's bucket, never under 256
/// (the cache holds one file per size, so few sizes is better).
fn dim(max_px: u32) -> u32 {
    match max_px {
        0..=256 => 256,
        257..=512 => 512,
        513..=1024 => 1024,
        _ => 2048,
    }
}

type Movies = Mutex<HashMap<(EntityId, u64), Arc<Vec<Vec<u8>>>>>;

fn load(b: &Backend, movies: &Movies, r: &Request<Src>) -> Result<ColorImage, String> {
    let s = &r.src;
    match s.what {
        What::Sequence => {
            let jpg = b.playback_frame(s.id, r.frame as usize, dim(r.max_px), s.color, 0.0)?;
            decode(&jpg, r.max_px)
        }
        What::Still => {
            if r.max_px <= 256 {
                if let Some(png) = b.thumbnail(s.id, s.color)? {
                    return decode(&png, r.max_px);
                }
            }
            let jpg = b.playback_frame(s.id, 0, dim(r.max_px), s.color, 0.0)?;
            decode(&jpg, r.max_px)
        }
        What::Movie => {
            let key = (s.id, s.key);
            let frames = {
                let got = movies.lock().unwrap().get(&key).cloned();
                match got {
                    Some(f) => f,
                    None => {
                        let f = Arc::new(b.scrub_thumbnails(s.id, MOVIE_STEPS as usize, s.color)?);
                        movies.lock().unwrap().insert(key, f.clone());
                        f
                    }
                }
            };
            let png = frames.get(r.frame as usize).filter(|p| !p.is_empty()).ok_or("no frame")?;
            decode(png, r.max_px)
        }
    }
}
