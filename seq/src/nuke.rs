//! "Copy for Nuke": the `::elementSelection::` clipboard protocol, and the
//! Nuke-side paste handler (taken live from the Svelte app's `nuke.ts`).

use seq_core::thumbs::InputSpace;
use serde_json::{json, Value};

use crate::model::{in_list, Kind, Row, MOVIE_EXTS};

pub const PREFIX: &str = "::elementSelection::";

fn colorspace(ext: &str, input: InputSpace) -> &'static str {
    if ext == "exr" {
        return if input == InputSpace::AcesCg { "ACEScg" } else { "linear" };
    }
    if in_list(ext, MOVIE_EXTS) {
        return "rec709";
    }
    "sRGB"
}

fn element(r: &Row, input: InputSpace, fps: u32) -> Option<Value> {
    match (&r.seq, r.kind) {
        (Some(s), _) => {
            let pat = if s.padding > 0 { format!("%0{}d", s.padding) } else { "%d".into() };
            let file = format!("{}{}{}{}.{}", s.prefix, s.delimiter.as_deref().unwrap_or(""), pat, s.suffix.as_deref().unwrap_or(""), s.extension);
            let printf = r.dir.join(file).display().to_string().replace('\\', "/");
            let mut e = json!({
                "name": s.name, "printf": printf, "kind": "sequence",
                "first": s.first, "last": s.last,
                "colorspace": colorspace(&s.extension.to_lowercase(), input), "fps": fps,
            });
            if !s.missing_frames.is_empty() {
                e["missing"] = json!(s.missing_frames);
            }
            Some(e)
        }
        (None, Kind::File) => {
            let ext = r.ext();
            let movie = in_list(&ext, MOVIE_EXTS);
            let mut e = json!({
                "name": r.name, "printf": r.path.display().to_string().replace('\\', "/"),
                "kind": if movie { "movie" } else { "still" }, "colorspace": colorspace(&ext, input),
            });
            if movie {
                e["fps"] = json!(fps);
            }
            Some(e)
        }
        _ => None,
    }
}

/// The clipboard text for these rows (non-media skipped), and how many went in.
pub fn clipboard(rows: &[&Row], input: InputSpace) -> (String, usize) {
    let els: Vec<Value> = rows.iter().filter_map(|r| element(r, input, 24)).collect();
    let n = els.len();
    (format!("{PREFIX}{}", json!({ "version": 1, "elements": els })), n)
}

/// The Python handler the user runs once in Nuke.
pub fn handler() -> &'static str {
    const TS: &str = include_str!("../../../sequence_manipulator/src/lib/nuke.ts");
    let start = TS.find("String.raw`").map(|i| i + "String.raw`".len()).unwrap_or(0);
    let end = TS.rfind("`;").unwrap_or(TS.len());
    &TS[start..end]
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_handler_is_the_python_out_of_nuke_ts() {
        let h = super::handler();
        assert!(h.starts_with("\"\"\"seq_manip -> Nuke"), "{}", &h[..40]);
        assert!(h.trim_end().ends_with("install()"));
    }
}
