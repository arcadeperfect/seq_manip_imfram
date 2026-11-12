//! seq_manip on imfram as a library: the whole app (`app::Seq` and its
//! panels) for the `seq_manip` binary, and for a host that embeds the
//! browser (Edify's clip browser). A host sets `Seq::embedded`: then
//! activating a sequence or movie (Enter, double-click) queues its media
//! path in `Seq::opened` for the host instead of playing it here.

pub mod app;
pub mod browser;
pub mod embed;
pub mod export;
pub mod jobs;
pub mod keys;
pub mod logic;
pub mod menus;
pub mod model;
pub mod nuke;
pub mod panels;
pub mod player;
pub mod prefs;
pub mod thumbs;
pub mod verbs;

use std::path::PathBuf;

/// The file a host opens for a row: a sequence's first frame on disk, a
/// movie or image itself; `None` for a folder.
pub fn media_path(r: &model::Row) -> Option<PathBuf> {
    match (&r.seq, r.kind) {
        (Some(s), _) => {
            let n = if s.padding > 0 { format!("{:0w$}", s.first, w = s.padding) } else { s.first.to_string() };
            Some(r.dir.join(format!("{}{}{}{}.{}", s.prefix, s.delimiter.as_deref().unwrap_or(""), n, s.suffix.as_deref().unwrap_or(""), s.extension)))
        }
        (None, model::Kind::File) => Some(r.path.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Kind, Life, Row};

    fn row(kind: Kind, seq: Option<model::Seq>) -> Row {
        Row { id: 1, kind, life: Life::Unchanged, hidden: false, seq, name: "x".into(), path: "/d/clip.mov".into(), dir: "/d".into(), pending: None, conflicts: vec![], broken: None, meta: None }
    }

    #[test]
    fn a_host_opens_a_sequences_first_frame_or_the_file() {
        let s = model::Seq { name: "shot".into(), prefix: "shot".into(), delimiter: Some("_".into()), suffix: None, extension: "exr".into(), first: 1001, last: 1100, on_disk: 100, missing: 0, missing_frames: vec![], padding: 4 };
        assert_eq!(media_path(&row(Kind::Sequence, Some(s))), Some(PathBuf::from("/d/shot_1001.exr")));
        assert_eq!(media_path(&row(Kind::File, None)), Some(PathBuf::from("/d/clip.mov")));
        assert_eq!(media_path(&row(Kind::Directory, None)), None);
    }
}
