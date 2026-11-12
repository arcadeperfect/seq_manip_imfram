//! The browser's decisions without a UI, each tested: which rows show and in
//! what order, tab completion, type-ahead, a scrub frame's number.

use std::collections::BTreeSet;

use crate::model::{Category, Kind, Row};
use crate::prefs::SortKey;

/// Rows that pass the name filter and the Filters menu, sorted (folders
/// first). `keep_order`: search results keep the scan's order.
pub fn order(rows: &[Row], filter: &str, shown: &BTreeSet<Category>, image_exts: &str, movie_exts: &str, sort: SortKey, ascending: bool, keep_order: bool) -> Vec<usize> {
    let f = filter.trim().to_lowercase();
    let mut out: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            let cat = r.category(image_exts, movie_exts);
            let hidden_ok = !r.hidden || shown.contains(&Category::Hidden);
            let cat_ok = if r.hidden { hidden_ok } else { shown.contains(&cat) };
            cat_ok && (f.is_empty() || r.name.to_lowercase().contains(&f))
        })
        .collect();
    if keep_order {
        return out;
    }
    out.sort_by(|&a, &b| {
        let (ra, rb) = (&rows[a], &rows[b]);
        let dirs = (rb.kind == Kind::Directory).cmp(&(ra.kind == Kind::Directory));
        if dirs != std::cmp::Ordering::Equal {
            return dirs;
        }
        let o = match sort {
            SortKey::Name => imfram::gallery::files::natural(&ra.name.to_lowercase(), &rb.name.to_lowercase()),
            SortKey::Size => ra.meta.as_ref().and_then(|m| m.size).cmp(&rb.meta.as_ref().and_then(|m| m.size)),
            SortKey::Modified => ra.meta.as_ref().and_then(|m| m.mtime).cmp(&rb.meta.as_ref().and_then(|m| m.mtime)),
            SortKey::Owner => ra.meta.as_ref().and_then(|m| m.owner.clone()).cmp(&rb.meta.as_ref().and_then(|m| m.owner.clone())),
        };
        let o = o.then_with(|| imfram::gallery::files::natural(&ra.name.to_lowercase(), &rb.name.to_lowercase()));
        if ascending {
            o
        } else {
            o.reverse()
        }
    });
    out
}

/// The longest prefix all candidates share (case-sensitive).
pub fn common_prefix(cands: &[String]) -> String {
    let Some(first) = cands.first() else { return String::new() };
    let mut end = first.len();
    for c in &cands[1..] {
        end = end.min(first.bytes().zip(c.bytes()).take_while(|(a, b)| a == b).count());
    }
    while !first.is_char_boundary(end) {
        end -= 1;
    }
    first[..end].to_string()
}

/// Terminal-style completion of `typed` from `cands`: a single match gets a
/// trailing slash; several complete to what they share. None: nothing to add
/// (the caller starts cycling).
pub fn complete(typed: &str, cands: &[String]) -> Option<String> {
    match cands {
        [] => None,
        [one] => Some(format!("{}/", one.trim_end_matches('/'))),
        many => {
            let p = common_prefix(many);
            (p.len() > typed.len()).then_some(p)
        }
    }
}

/// The first shown row (in `order`) whose name starts with `typed`.
pub fn typeahead(rows: &[Row], order: &[usize], typed: &str) -> Option<usize> {
    let t = typed.to_lowercase();
    order.iter().position(|&i| rows[i].name.to_lowercase().starts_with(&t))
}

/// The frame number of the `n`th frame on disk, given the span's first
/// frame and its (sorted) missing frames.
pub fn nth_frame(first: i32, missing: &[i32], n: usize) -> i32 {
    let mut f = first + n as i32;
    for &m in missing {
        if m <= f {
            f += 1;
        } else {
            break;
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_takes_the_shared_prefix_and_a_slash_for_one() {
        let c = vec!["/a/renders".to_string(), "/a/reports".to_string()];
        assert_eq!(complete("/a/r", &c), Some("/a/re".into()));
        assert_eq!(complete("/a/re", &c), None);
        assert_eq!(complete("/a/ren", &["/a/renders".to_string()]), Some("/a/renders/".into()));
    }

    #[test]
    fn a_frame_number_steps_over_the_holes() {
        // 1..12 with 5, 9, 10 missing: on disk 1 2 3 4 6 7 8 11 12
        let m = [5, 9, 10];
        assert_eq!(nth_frame(1, &m, 0), 1);
        assert_eq!(nth_frame(1, &m, 4), 6);
        assert_eq!(nth_frame(1, &m, 7), 11);
        assert_eq!(nth_frame(1, &m, 8), 12);
    }
}
