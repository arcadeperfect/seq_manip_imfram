//! The app's state without a UI: the registry, the folder open, and the
//! rows a view draws. Mirrors `commands.rs` call for call, minus IPC.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sequitur::{EntryKind, FileSequence, OperationPlan};

use crate::state::{Canonical, Entity, EntityId, IntentOp, Lifecycle, Projected, Registry};

/// One entity as a view draws it: the projected look plus the diff.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: EntityId,
    pub kind: Kind,
    /// Projected name: `shot_####.exr` or `notes.txt`.
    pub name: String,
    pub lifecycle: Lifecycle,
    pub hidden: bool,
    /// Sequences: first, last, frames on disk, missing, padding.
    pub seq: Option<SeqInfo>,
    /// Paths on disk now (canonical), first frame first: what thumbnails
    /// read. A staged copy shows its source's frames.
    pub frames: Arc<Vec<PathBuf>>,
    pub dir: PathBuf,
    /// Staged edits, one line each, in stage order (index = revert index).
    pub intents: Vec<String>,
    /// `before → after` when anything is staged.
    pub before: String,
    pub after: String,
    pub conflicts: Vec<PathBuf>,
    /// The staged edits no longer fold over what is on disk.
    pub broken: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Sequence,
    File,
    Directory,
    Symlink,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Sequence => "sequence",
            Kind::File => "file",
            Kind::Directory => "folder",
            Kind::Symlink => "link",
            Kind::Other => "other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SeqInfo {
    pub prefix: String,
    pub delimiter: Option<String>,
    pub suffix: Option<String>,
    pub extension: String,
    pub first: i32,
    pub last: i32,
    pub on_disk: usize,
    pub missing: u64,
    pub padding: usize,
}

impl SeqInfo {
    fn of(seq: &FileSequence) -> Self {
        Self {
            prefix: seq.prefix().unwrap_or_default().to_string(),
            delimiter: seq.delimiter().ok().flatten().map(String::from),
            suffix: seq.suffix().ok().flatten().map(String::from),
            extension: seq.extension().unwrap_or_default().to_string(),
            first: seq.first_frame(),
            last: seq.last_frame(),
            on_disk: seq.actual_frame_count(),
            missing: seq.missing_frame_count(),
            padding: seq.padding().unwrap_or(0),
        }
    }
}

fn kind_of(k: EntryKind) -> Kind {
    match k {
        EntryKind::File => Kind::File,
        EntryKind::Directory => Kind::Directory,
        EntryKind::Symlink => Kind::Symlink,
        EntryKind::Other => Kind::Other,
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn seq_name(seq: &FileSequence) -> String {
    seq.sequence_string().unwrap_or_else(|_| "<sequence>".into())
}

/// Build one row (never fails: a broken entity shows as it is on disk).
pub fn row(id: EntityId, e: &Entity, conflicts: &HashMap<EntityId, Vec<PathBuf>>) -> Row {
    let dir = e.canonical_dir().unwrap_or_default();
    let (frames, hidden) = match &e.canonical {
        Canonical::Sequence(s) => (s.items().iter().map(|i| i.path()).collect(), false),
        Canonical::Loose { path, hidden, .. } => (vec![path.clone()], *hidden),
    };
    let intents: Vec<String> = e.intents.iter().map(IntentOp::summary).collect();
    let before = if e.is_copy() { format!("copy of {}", e.canonical_name()) } else { e.canonical_name() };
    let conflicts = conflicts.get(&id).cloned().unwrap_or_default();
    let canonical_look = |c: &Canonical| match c {
        Canonical::Sequence(s) => (Kind::Sequence, seq_name(s), Some(SeqInfo::of(s))),
        Canonical::Loose { path, kind, .. } => (kind_of(*kind), file_name(path), None),
    };
    match e.project() {
        Err(err) => {
            let (kind, name, seq) = canonical_look(&e.canonical);
            Row { id, kind, name, lifecycle: Lifecycle::Modified, hidden, seq, frames: Arc::new(frames), dir, intents, before, after: "(cannot apply)".into(), conflicts, broken: Some(err.to_string()) }
        }
        Ok(p) => {
            let lifecycle = e.lifecycle(&p);
            let (kind, name, seq) = match (&e.canonical, &p.projected) {
                (Canonical::Sequence(_), Projected::Sequence(s)) => (Kind::Sequence, seq_name(s), Some(SeqInfo::of(s))),
                (Canonical::Loose { kind, .. }, Projected::Loose(path)) => (kind_of(*kind), file_name(path), None),
                _ => canonical_look(&e.canonical),
            };
            let after = if p.deleted { "(deleted)".into() } else { name.clone() };
            Row { id, kind, name, lifecycle, hidden, seq, frames: Arc::new(frames), dir, intents, before, after, conflicts, broken: None }
        }
    }
}

/// What a commit did.
#[derive(Clone, Debug, Default)]
pub struct CommitOutcome {
    pub committed: bool,
    pub executed: usize,
    pub cancelled: bool,
    pub skipped: usize,
    pub error: Option<String>,
}

/// A commit's work, taken from the registry so it can run off the UI
/// thread. `finish` hands the result back.
pub struct CommitJob {
    pub plan: OperationPlan,
    pub ids: Vec<EntityId>,
    pub skipped: usize,
}

/// One executor step.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub index: usize,
    pub total: usize,
    pub label: String,
}

impl CommitJob {
    /// Run the plan (blocking). `step` sees each operation.
    pub fn run(self, force: bool, cancel: &AtomicBool, mut step: impl FnMut(Progress)) -> (Vec<EntityId>, CommitOutcome) {
        let skipped = self.skipped;
        let result = self.plan.commit_observed(force, |p| {
            let label = p.operation.destination().unwrap_or_else(|| p.operation.source()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            step(Progress { index: p.index, total: p.total, label });
            if cancel.load(Ordering::SeqCst) {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        });
        let cancelled = cancel.load(Ordering::SeqCst);
        match result {
            Err(e) => (self.ids, CommitOutcome { committed: false, executed: 0, cancelled, skipped, error: Some(e.to_string()) }),
            Ok(r) => {
                let ok = r.success() && !cancelled;
                let error = r.failed.first().map(|(op, e)| format!("{}: {e}", op.source().display()));
                (self.ids, CommitOutcome { committed: ok, executed: r.executed.len(), cancelled, skipped, error })
            }
        }
    }
}

/// A folder (or the bin) as rows, over the session registry.
#[derive(Default)]
pub struct Session {
    pub registry: Registry,
    /// The folder shown.
    pub dir: Option<PathBuf>,
    /// Its entities, in scan order (sequences first), then staged copies landing here.
    pub ids: Vec<EntityId>,
    pub rows: Vec<Row>,
    /// Names whose staged edits an outside change voided, since last asked.
    pub voided: Vec<String>,
    /// Bumped whenever `rows` changes.
    pub gen: u64,
    /// Fewer frames than this and a group shows as loose files.
    pub min_frames: usize,
    /// True while a commit runs: staging is refused.
    pub busy: bool,
}

pub const BUSY: &str = "a commit is in progress; try again when it finishes";

impl Session {
    pub fn new() -> Self {
        Self { min_frames: 2, ..Default::default() }
    }

    /// Scan `dir` into the registry and show it.
    pub fn open(&mut self, dir: &Path) -> Result<(), String> {
        if self.busy {
            return Err(BUSY.into());
        }
        let dir = dunce::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let listing = FileSequence::parse_directory(&dir, self.min_frames.max(1)).map_err(|e| e.to_string())?;
        let entries = listing.entries.iter().map(|e| (e.path.clone(), e.kind, e.hidden)).collect();
        let out = self.registry.absorb_scan(&dir, listing.sequences, entries);
        let mut ids = out.ids;
        ids.extend(self.registry.copies_in(&dir));
        self.voided.extend(out.voided);
        self.ids = ids;
        self.dir = Some(dir);
        self.rebuild();
        Ok(())
    }

    /// Scan the folder again (after a commit, or an outside change).
    pub fn refresh(&mut self) -> Result<(), String> {
        match self.dir.clone() {
            Some(d) => self.open(&d),
            None => Ok(()),
        }
    }

    /// Recompute rows from the registry.
    pub fn rebuild(&mut self) {
        let conflicts = self.registry.projected_conflicts();
        self.rows = self.ids.iter().filter_map(|&id| self.registry.get(id).map(|e| row(id, e, &conflicts))).collect();
        self.gen += 1;
    }

    pub fn row(&self, id: EntityId) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// Every entity with staged edits, across folders.
    pub fn pending(&self) -> Vec<Row> {
        let conflicts = self.registry.projected_conflicts();
        self.registry.pending().into_iter().map(|(id, e)| row(id, e, &conflicts)).collect()
    }

    pub fn stage(&mut self, id: EntityId, intent: IntentOp) -> Result<(), String> {
        if self.busy {
            return Err(BUSY.into());
        }
        self.registry.stage_intent(id, intent)?;
        self.rebuild();
        Ok(())
    }

    /// A staged copy of `id`: new prefix (sequences) or name (files), and/or a new folder.
    pub fn stage_copy(&mut self, id: EntityId, dest: Option<PathBuf>, prefix: Option<String>, name: Option<String>) -> Result<EntityId, String> {
        if self.busy {
            return Err(BUSY.into());
        }
        let copy = self.registry.stage_copy(id, dest, prefix, name)?;
        if let Some(dir) = &self.dir {
            if self.registry.copies_in(dir).contains(&copy) && !self.ids.contains(&copy) {
                self.ids.push(copy);
            }
        }
        self.rebuild();
        Ok(copy)
    }

    pub fn revert(&mut self, id: EntityId, index: usize) -> Result<(), String> {
        if self.busy {
            return Err(BUSY.into());
        }
        if self.registry.revert_intent(id, index)? {
            self.ids.retain(|&i| i != id);
        }
        self.rebuild();
        Ok(())
    }

    pub fn clear(&mut self, id: EntityId) -> Result<(), String> {
        if self.busy {
            return Err(BUSY.into());
        }
        if self.registry.clear_intents(id)? {
            self.ids.retain(|&i| i != id);
        }
        self.rebuild();
        Ok(())
    }

    /// Take the commit's work (all pending, or just `only`). Errors when
    /// there is nothing that can be committed.
    pub fn begin_commit(&mut self, only: Option<&[EntityId]>) -> Result<CommitJob, String> {
        if self.busy {
            return Err(BUSY.into());
        }
        let mut plan = OperationPlan::new();
        let mut ids = Vec::new();
        let mut skipped = 0;
        for (id, e) in self.registry.pending() {
            if only.is_some_and(|o| !o.contains(&id)) {
                continue;
            }
            match e.commit_plan() {
                Ok(p) => {
                    plan.extend(p);
                    ids.push(id);
                }
                Err(_) => skipped += 1,
            }
        }
        if ids.is_empty() {
            return Err(if skipped > 0 { "staged changes can't be applied (broken by an outside change); revert them".into() } else { "nothing staged to commit".into() });
        }
        self.busy = true;
        Ok(CommitJob { plan, ids, skipped })
    }

    /// Hand a finished commit back: finalize on success, else drop the
    /// targets' edits (some may be on disk). Then scan again.
    pub fn finish_commit(&mut self, ids: &[EntityId], out: &CommitOutcome) {
        self.busy = false;
        if out.committed {
            self.registry.finalize_commit(ids);
        } else {
            for &id in ids {
                let _ = self.registry.clear_intents(id);
            }
        }
        // Copies now exist on disk under their own names; drop the virtual ids.
        let _ = self.refresh();
    }

    /// Commit now, on this thread (Instant mode, tests).
    pub fn commit_now(&mut self, only: Option<&[EntityId]>) -> Result<CommitOutcome, String> {
        let job = self.begin_commit(only)?;
        let cancel = AtomicBool::new(false);
        let (ids, out) = job.run(false, &cancel, |_| {});
        self.finish_commit(&ids, &out);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::NameComponents;

    fn fixture(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seqcore-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in 1..=5 {
            std::fs::write(d.join(format!("shot.{f:04}.exr")), b"x").unwrap();
        }
        std::fs::write(d.join("notes.txt"), b"hi").unwrap();
        d
    }

    #[test]
    fn a_folder_shows_one_row_per_sequence_and_a_staged_rename_commits() {
        let d = fixture("rename");
        let mut s = Session::new();
        s.open(&d).unwrap();
        assert_eq!(s.rows.len(), 2, "{:?}", s.rows.iter().map(|r| &r.name).collect::<Vec<_>>());
        let seq = s.rows.iter().find(|r| r.kind == Kind::Sequence).unwrap().clone();
        assert_eq!(seq.frames.len(), 5);
        s.stage(seq.id, IntentOp::Rename { to: NameComponents { prefix: Some("comp".into()), delimiter: None, suffix: None, extension: None, padding: None } }).unwrap();
        assert_eq!(s.pending().len(), 1);
        assert!(s.row(seq.id).unwrap().name.starts_with("comp"));
        let out = s.commit_now(None).unwrap();
        assert!(out.committed, "{out:?}");
        assert!(d.join("comp.0001.exr").exists());
        assert!(s.pending().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
