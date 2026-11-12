//! An entity as the views draw it: the app's `EntityView`, owned and
//! cloneable, plus the metadata that arrives later.

use std::path::{Path, PathBuf};

use seq_core::backend::EntityView;
use seq_core::state::EntityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sequence,
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    Unchanged,
    Modified,
    Deleted,
    Departing,
    New,
}

#[derive(Clone, Debug)]
pub struct Seq {
    pub name: String,
    pub prefix: String,
    pub delimiter: Option<String>,
    pub suffix: Option<String>,
    pub extension: String,
    pub first: i32,
    pub last: i32,
    pub on_disk: usize,
    pub missing: u64,
    pub missing_frames: Vec<i32>,
    pub padding: usize,
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub before: String,
    pub after: String,
    pub intents: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub size: Option<u64>,
    pub mtime: Option<i64>,
    pub owner: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub id: EntityId,
    pub kind: Kind,
    pub life: Life,
    pub hidden: bool,
    pub seq: Option<Seq>,
    /// Projected name: `shot_####.exr`, `notes.txt`.
    pub name: String,
    /// A loose entry's projected path; a sequence's folder + name.
    pub path: PathBuf,
    pub dir: PathBuf,
    pub pending: Option<Pending>,
    pub conflicts: Vec<String>,
    pub broken: Option<String>,
    pub meta: Option<Meta>,
}

pub const IMAGE_EXTS: &str = "exr,png,jpg,jpeg,tif,tiff,gif,bmp,webp,dpx";
pub const MOVIE_EXTS: &str = "mov,mp4,m4v,avi,mkv,webm,mxf";

pub fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn in_list(ext: &str, list: &str) -> bool {
    !ext.is_empty() && list.split(',').any(|e| e.trim().trim_start_matches('.').eq_ignore_ascii_case(ext))
}

impl Row {
    pub fn from_view(v: EntityView) -> Self {
        let kind = match v.kind.as_str() {
            "sequence" => Kind::Sequence,
            "file" => Kind::File,
            "directory" => Kind::Directory,
            "symlink" => Kind::Symlink,
            _ => Kind::Other,
        };
        let life = match v.lifecycle.as_str() {
            "modified" => Life::Modified,
            "deleted" => Life::Deleted,
            "departing" => Life::Departing,
            "new" => Life::New,
            _ => Life::Unchanged,
        };
        let dir = PathBuf::from(&v.dir);
        let seq = v.sequence.map(|s| Seq {
            name: s.name,
            prefix: s.prefix,
            delimiter: s.delimiter,
            suffix: s.suffix,
            extension: s.extension,
            first: s.first_frame,
            last: s.last_frame,
            on_disk: s.actual_frame_count,
            missing: s.missing_frame_count,
            missing_frames: s.missing_frames,
            padding: s.padding,
        });
        let (name, path) = match (&seq, v.entry) {
            (Some(s), _) => (s.name.clone(), dir.join(&s.name)),
            (None, Some(e)) => (e.name, PathBuf::from(e.path)),
            (None, None) => (String::new(), dir.clone()),
        };
        Row {
            id: v.id,
            kind,
            life,
            hidden: v.hidden,
            seq,
            name,
            path,
            dir,
            pending: v.pending.map(|p| Pending { before: p.before_summary, after: p.after_summary, intents: p.intents }),
            conflicts: v.conflicts,
            broken: if v.broken { Some(v.broken_reason.unwrap_or_default()) } else { None },
            meta: None,
        }
    }

    pub fn ext(&self) -> String {
        match &self.seq {
            Some(s) => s.extension.to_lowercase(),
            None => ext_of(&self.name),
        }
    }

    pub fn is_movie(&self) -> bool {
        self.kind == Kind::File && in_list(&self.ext(), MOVIE_EXTS)
    }

    /// The name on disk now (before staged edits).
    pub fn canonical_name(&self) -> &str {
        match &self.pending {
            Some(p) if !p.before.starts_with("copy of ") => &p.before,
            _ => &self.name,
        }
    }

    /// Frames a scrub reaches (1: a still).
    pub fn frames(&self) -> usize {
        self.seq.as_ref().map(|s| s.on_disk).unwrap_or(1)
    }

    /// `shot.####.exr` for sequences, the path otherwise.
    pub fn hash_path(&self) -> String {
        self.path.display().to_string()
    }

    pub fn status(&self) -> &'static str {
        if self.broken.is_some() {
            return "can't apply";
        }
        if !self.conflicts.is_empty() {
            return "conflict";
        }
        match self.life {
            Life::Unchanged => "",
            Life::Modified => "modified",
            Life::Deleted => "deleted",
            Life::Departing => "departing",
            Life::New => "new",
        }
    }

    /// One of the Filters menu's categories.
    pub fn category(&self, image_exts: &str, movie_exts: &str) -> Category {
        if self.hidden {
            return Category::Hidden;
        }
        match self.kind {
            Kind::Sequence if in_list(&self.ext(), image_exts) => Category::ImageSeq,
            Kind::Sequence => Category::OtherSeq,
            Kind::Directory => Category::Folder,
            Kind::File if in_list(&self.ext(), movie_exts) => Category::Movie,
            Kind::File if in_list(&self.ext(), image_exts) => Category::Image,
            _ => Category::OtherFile,
        }
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Category {
    ImageSeq,
    OtherSeq,
    Movie,
    Image,
    OtherFile,
    Folder,
    Hidden,
}

impl Category {
    pub const ALL: [Category; 7] = [Category::ImageSeq, Category::OtherSeq, Category::Movie, Category::Image, Category::OtherFile, Category::Folder, Category::Hidden];
    pub fn label(self) -> &'static str {
        match self {
            Category::ImageSeq => "image sequences",
            Category::OtherSeq => "other sequences",
            Category::Movie => "movies",
            Category::Image => "image files",
            Category::OtherFile => "other files",
            Category::Folder => "folders",
            Category::Hidden => "hidden files",
        }
    }
}

pub fn human_bytes(n: u64) -> String {
    imfram::gallery::files::size_text(n)
}

/// `YYYY-MM-DD HH:MM` in local time.
pub fn date_text(secs: i64) -> String {
    // No chrono: civil-from-days on the local offset the OS reports.
    let off = local_offset();
    let t = secs + off;
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn local_offset() -> i64 {
    use std::sync::OnceLock;
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let (sign, rest) = s.split_at(1);
                let h: i64 = rest.get(0..2)?.parse().ok()?;
                let m: i64 = rest.get(2..4)?.parse().ok()?;
                Some(if sign == "-" { -1 } else { 1 } * (h * 3600 + m * 60))
            })
            .unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_the_calendar_does() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
    }
}
