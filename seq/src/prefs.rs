//! What the app remembers between runs (the Svelte app's `seqmanip.*`
//! localStorage keys), as one JSON file, plus the dock layout.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{Category, IMAGE_EXTS, MOVIE_EXTS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Owner,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Owner];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
            SortKey::Owner => "owner",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum InputPref {
    #[default]
    Linear,
    AcesCg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewPref {
    #[default]
    Aces,
    Srgb,
    Raw,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub favourites: Vec<PathBuf>,
    pub view: ViewMode,
    pub grid_size: f32,
    pub row_h: f32,
    pub bin_view: ViewMode,
    pub shown: BTreeSet<Category>,
    pub image_exts: String,
    pub movie_exts: String,
    pub input: InputPref,
    pub view_transform: ViewPref,
    pub sort: SortKey,
    pub ascending: bool,
    pub instant: bool,
    pub zoom: f32,
    pub light: bool,
    pub last_dir: Option<PathBuf>,
    // one-time notices
    pub warned_instant: bool,
    pub warned_commit: bool,
    pub warned_delete: bool,
    pub splash_seen: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            favourites: vec![],
            view: ViewMode::Grid,
            grid_size: 200.0,
            row_h: 36.0,
            bin_view: ViewMode::List,
            shown: Category::ALL.into_iter().filter(|c| *c != Category::Hidden).collect(),
            image_exts: IMAGE_EXTS.into(),
            movie_exts: MOVIE_EXTS.into(),
            input: InputPref::Linear,
            view_transform: ViewPref::Aces,
            sort: SortKey::Name,
            ascending: true,
            instant: false,
            zoom: 1.0,
            light: false,
            last_dir: None,
            warned_instant: false,
            warned_commit: false,
            warned_delete: false,
            splash_seen: false,
        }
    }
}

pub fn dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    #[cfg(target_os = "macos")]
    return home.join("Library/Application Support/seq_manip_imfram");
    #[cfg(not(target_os = "macos"))]
    return home.join(".config/seq_manip_imfram");
}

impl Prefs {
    pub fn load() -> Self {
        std::fs::read_to_string(dir().join("prefs.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let d = dir();
        let _ = std::fs::create_dir_all(&d);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = d.join("prefs.json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, d.join("prefs.json"));
            }
        }
    }
    pub fn color(&self) -> seq_core::backend::Color {
        use seq_core::thumbs::{InputSpace, ViewTransform};
        seq_core::backend::Color {
            input: match self.input {
                InputPref::Linear => InputSpace::LinearSrgb,
                InputPref::AcesCg => InputSpace::AcesCg,
            },
            view: match self.view_transform {
                ViewPref::Aces => ViewTransform::Aces,
                ViewPref::Srgb => ViewTransform::Gamma22,
                ViewPref::Raw => ViewTransform::Raw,
            },
        }
    }
}
