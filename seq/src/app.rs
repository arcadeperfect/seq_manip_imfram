//! The app: its tabs and windows, and the state they share. Every change
//! goes through the backend (seq_manip's own commands); views are rebuilt
//! from what it answers.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::Ui;
use imfram::dock::{self, Panels};
use imfram::toast::{Kind as ToastKind, Toasts};
use seq_core::backend::{Backend, DirView};
use seq_core::state::{EntityId, IntentOp};

use crate::browser::Browser;
use crate::jobs::{Done, Jobs, Long};
use crate::model::{Kind, Life, Meta, Row};
use crate::player::Player;
use crate::prefs::Prefs;
use crate::verbs::Verb;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Folders,
    Browser,
    Info,
    Pending,
    Bin,
    Deletes,
    Player,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Role {
    Main,
    Detached,
}

impl dock::Tab for Tab {
    type Role = Role;
    fn all() -> &'static [Self] {
        &[Tab::Folders, Tab::Browser, Tab::Info, Tab::Pending, Tab::Bin, Tab::Deletes, Tab::Player]
    }
    fn title(self) -> &'static str {
        match self {
            Tab::Folders => "Folders",
            Tab::Browser => "Browser",
            Tab::Info => "Info",
            Tab::Pending => "Pending",
            Tab::Bin => "Bin",
            Tab::Deletes => "Delete queue",
            Tab::Player => "Player",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Tab::Folders => "favourites and the folder tree",
            Tab::Browser => "the folder (or search, bin, delete review): sequences as single items",
            Tab::Info => "the selected item: media facts and staged edits",
            Tab::Pending => "every staged edit, ready to commit",
            Tab::Bin => "items gathered from anywhere to work on together",
            Tab::Deletes => "items queued for deletion, reviewed then deleted together",
            Tab::Player => "plays a sequence or movie (Space / J K L, F fit, 1 1:1)",
        }
    }
    fn home(self) -> Role {
        Role::Main
    }
}

impl dock::Role for Role {
    fn title(self) -> &'static str {
        "seq_manip"
    }
    fn size(self) -> (u32, u32) {
        match self {
            Role::Main => (1500, 900),
            Role::Detached => (900, 640),
        }
    }
    fn detached() -> Self {
        Role::Detached
    }
}

/// What the browser lists.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Dir(PathBuf),
    Search { root: PathBuf, query: String, ids: Vec<EntityId> },
    Bin,
    Deletes,
}

/// Something the main loop does to the dock.
pub enum DockAsk {
    Show(Tab),
}

/// A question over the window.
pub enum Ask {
    /// First commit: files will now change.
    CommitWarn(Option<Vec<EntityId>>),
    /// Some deletes can't go to a trash.
    Permanent { ids: Option<Vec<EntityId>>, count: usize, dirs: Vec<String> },
    /// First switch to Instant.
    InstantWarn,
    /// First staged delete.
    DeleteWarn(Vec<EntityId>),
    /// Run the delete queue.
    ExecuteDeletes { to_trash: bool },
    Splash,
    /// The Stage / Instant explainer.
    ModeHelp,
}

/// An item's media facts, owned.
#[derive(Clone, Debug)]
pub struct Info {
    pub format: String,
    pub path: String,
    pub fields: Vec<(String, String)>,
}

pub struct Seq {
    pub jobs: Jobs,
    pub prefs: Prefs,
    pub source: Source,
    /// The folder the browser was last on (search roots, Back from the bin).
    pub dir: PathBuf,
    pub rows: Vec<Row>,
    pub gen: u64,
    pub selection: BTreeSet<u64>,
    pub browser: Browser,
    pub voided: Vec<String>,
    pub pending: Vec<Row>,
    pub bin: Vec<EntityId>,
    pub bin_rows: Vec<Row>,
    pub bin_view: crate::panels::BinView,
    pub deletes: Vec<EntityId>,
    pub delete_rows: Vec<Row>,
    pub delete_trash: bool,
    pub delete_preview: Option<(usize, u64, Vec<String>)>,
    pub tree: crate::panels::Tree,
    pub info: HashMap<EntityId, Result<Info, String>>,
    pub meta: HashMap<EntityId, Meta>,
    meta_asked: BTreeSet<EntityId>,
    pub player: Player,
    pub verb: Option<Verb>,
    pub ask: Option<Ask>,
    pub export: Option<crate::export::Form>,
    pub toasts: Toasts,
    pub bar: imfram::tiltmenu::TiltBar,
    pub dock_asks: Vec<DockAsk>,
    pub search_progress: Option<String>,
    pub quit: bool,
    prefs_dirty: bool,
    /// Offscreen shots: nothing is written to the prefs.
    pub keep_prefs: bool,
}

impl Seq {
    pub fn new(dir: Option<PathBuf>) -> Self {
        let backend = Arc::new(Backend::new());
        let prefs = Prefs::load();
        let home = backend.home_directory().map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
        let start = dir.or_else(|| prefs.last_dir.clone().filter(|d| d.is_dir())).unwrap_or(home);
        let browser = Browser::new(backend.clone(), &prefs);
        let mut s = Self {
            jobs: Jobs::new(backend.clone()),
            source: Source::Dir(start.clone()),
            dir: start.clone(),
            rows: vec![],
            gen: 0,
            selection: BTreeSet::new(),
            browser,
            voided: vec![],
            pending: vec![],
            bin: vec![],
            bin_rows: vec![],
            bin_view: crate::panels::BinView::new(backend.clone()),
            deletes: vec![],
            delete_rows: vec![],
            delete_trash: true,
            delete_preview: None,
            tree: crate::panels::Tree::default(),
            info: HashMap::new(),
            meta: HashMap::new(),
            meta_asked: BTreeSet::new(),
            player: Player::new(backend),
            verb: None,
            ask: if prefs.splash_seen { None } else { Some(Ask::Splash) },
            export: None,
            toasts: Toasts::new(),
            bar: imfram::tiltmenu::TiltBar::default(),
            dock_asks: vec![],
            search_progress: None,
            quit: false,
            prefs_dirty: false,
            keep_prefs: false,
            prefs,
        };
        s.go(&start);
        s
    }

    pub fn backend(&self) -> &Backend {
        &self.jobs.backend
    }

    pub fn busy(&self) -> bool {
        self.jobs.long == Some(Long::Commit)
    }

    pub fn prefs_changed(&mut self) {
        self.prefs_dirty = true;
    }

    // ---- notices ----

    pub fn error(&mut self, e: impl Into<String>) {
        self.toasts.push(ToastKind::Error, e);
    }
    pub fn info_toast(&mut self, t: impl Into<String>) {
        self.toasts.push(ToastKind::Success, t);
    }
    pub fn warn(&mut self, t: impl Into<String>) {
        self.toasts.push(ToastKind::Warn, t);
    }

    // ---- what the browser shows ----

    /// Open a folder (a new place in history).
    pub fn go(&mut self, dir: &Path) {
        let before = self.source.clone();
        if self.show_source(Source::Dir(dir.to_path_buf())) {
            if before != self.source {
                self.browser.back.push(before);
                self.browser.fwd.clear();
            }
            self.selection.clear();
            self.browser.scrolled_to_top();
        }
    }

    /// Show a source; false (and a notice) if it could not be read.
    pub fn show_source(&mut self, src: Source) -> bool {
        match self.load(&src) {
            Ok(v) => {
                if let Source::Dir(_) = src {
                    let d = PathBuf::from(&v.path);
                    self.dir = d.clone();
                    self.prefs.last_dir = Some(d.clone());
                    self.prefs_changed();
                    self.source = Source::Dir(d.clone());
                    let b = self.jobs.backend.clone();
                    self.tree.reveal(&b, &d);
                } else {
                    self.source = src;
                }
                self.take_view(v);
                true
            }
            Err(e) => {
                self.error(e);
                false
            }
        }
    }

    fn load(&self, src: &Source) -> Result<DirView, String> {
        let b = self.backend();
        match src {
            Source::Dir(d) => b.open_directory(&d.to_string_lossy(), None),
            Source::Search { ids, root, .. } => b.view_entities(ids.clone()).map(|mut v| {
                v.path = root.display().to_string();
                v
            }),
            Source::Bin => b.view_entities(self.bin.clone()),
            Source::Deletes => b.view_entities(self.deletes.clone()),
        }
    }

    fn take_view(&mut self, v: DirView) {
        if !v.voided.is_empty() {
            self.warn(format!("staged edits dropped because the files changed on disk: {}", v.voided.join(", ")));
            self.voided = v.voided;
        }
        let mut rows: Vec<Row> = v.entities.into_iter().map(Row::from_view).collect();
        for r in &mut rows {
            r.meta = self.meta.get(&r.id).cloned();
        }
        self.rows = rows;
        self.selection.retain(|id| self.rows.iter().any(|r| r.id == *id));
        self.gen += 1;
        self.refresh_side();
    }

    /// Read the source again (after a stage, a commit, an outside change).
    pub fn reload(&mut self) {
        let src = self.source.clone();
        match self.load(&src) {
            Ok(v) => self.take_view(v),
            Err(e) => {
                self.error(e);
                self.refresh_side();
            }
        }
    }

    /// Pending, bin and delete queue, from the registry.
    pub fn refresh_side(&mut self) {
        self.pending = self.backend().pending_entities().unwrap_or_default().into_iter().map(Row::from_view).collect();
        let present = |rows: &[Row], id: EntityId| rows.iter().any(|r| r.id == id);
        let rows_of = |s: &Self, ids: &[EntityId]| -> Vec<Row> {
            if ids.is_empty() {
                return vec![];
            }
            match s.backend().view_entities(ids.to_vec()) {
                Ok(v) => v.entities.into_iter().map(Row::from_view).collect(),
                Err(_) => vec![],
            }
        };
        // The bin and the queue keep only items that still exist.
        if !self.bin.is_empty() {
            self.bin_rows = rows_of(self, &self.bin.clone());
            self.bin.retain(|id| present(&self.bin_rows, *id));
        } else {
            self.bin_rows.clear();
        }
        if !self.deletes.is_empty() {
            self.delete_rows = rows_of(self, &self.deletes.clone());
            self.deletes.retain(|id| present(&self.delete_rows, *id));
            self.delete_preview = self.backend().delete_preview(self.deletes.clone()).ok().map(|p| (p.files, p.bytes, p.no_trash_dirs));
        } else {
            self.delete_rows.clear();
            self.delete_preview = None;
        }
    }

    pub fn row(&self, id: EntityId) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id).or_else(|| self.bin_rows.iter().find(|r| r.id == id)).or_else(|| self.pending.iter().find(|r| r.id == id))
    }

    /// The selected rows, in listing order.
    pub fn selected(&self) -> Vec<&Row> {
        self.rows.iter().filter(|r| self.selection.contains(&r.id)).collect()
    }
    pub fn selected_ids(&self) -> Vec<EntityId> {
        self.selected().iter().map(|r| r.id).collect()
    }

    /// Ask for size / date / owner of rows that lack it.
    pub fn want_meta(&mut self) {
        let ids: Vec<EntityId> = self.rows.iter().filter(|r| r.meta.is_none() && !self.meta_asked.contains(&r.id)).map(|r| r.id).collect();
        if ids.is_empty() {
            return;
        }
        self.meta_asked.extend(ids.iter().copied());
        self.jobs.spawn(move |b, _| Done::Meta(b.entity_metadata(ids).unwrap_or_default()));
    }

    /// Media facts for one row (asked once).
    pub fn want_info(&mut self, id: EntityId) {
        if self.info.contains_key(&id) {
            return;
        }
        self.info.insert(id, Err("reading…".into()));
        self.jobs.spawn(move |b, _| Done::Info(id, b.media_info(id)));
    }

    // ---- edits ----

    /// Stage edits; in Instant mode commit them at once. Deletes ask once first.
    pub fn apply(&mut self, edits: Vec<(EntityId, IntentOp)>) {
        if edits.iter().any(|(_, op)| matches!(op, IntentOp::Delete)) && !self.prefs.warned_delete {
            let ids = edits.iter().map(|e| e.0).collect();
            self.ask = Some(Ask::DeleteWarn(ids));
            return;
        }
        let mut done = vec![];
        for (id, op) in edits {
            match self.backend().stage_intent(id, op) {
                Ok(_) => done.push(id),
                Err(e) => {
                    self.error(e);
                    break;
                }
            }
        }
        self.reload();
        if self.prefs.instant && !done.is_empty() {
            self.commit(Some(done));
        }
    }

    /// Staged copies (dest folder, new prefix / name per id).
    pub fn copy(&mut self, copies: Vec<(EntityId, Option<String>, Option<String>, Option<String>)>) {
        let mut made = vec![];
        for (id, dest, prefix, name) in copies {
            match self.backend().stage_copy(id, dest, prefix, name) {
                Ok(v) => made.push(v.id),
                Err(e) => {
                    self.error(e);
                    break;
                }
            }
        }
        self.reload();
        if self.prefs.instant && !made.is_empty() {
            self.commit(Some(made));
        }
    }

    pub fn revert(&mut self, id: EntityId, index: Option<usize>) {
        let r = match index {
            Some(i) => self.backend().revert_intent(id, i).map(|_| ()),
            None => self.backend().clear_intents(id).map(|_| ()),
        };
        if let Err(e) = r {
            self.error(e);
        }
        self.reload();
    }

    /// Commit (all, or `ids`), through the one-time warning and the
    /// no-trash check.
    pub fn commit(&mut self, ids: Option<Vec<EntityId>>) {
        if self.jobs.long.is_some() {
            return self.warn("wait for the running job to finish");
        }
        if !self.prefs.warned_commit {
            self.ask = Some(Ask::CommitWarn(ids));
            return;
        }
        match self.backend().permanent_deletes(ids.clone()) {
            Ok(p) if p.count > 0 => self.ask = Some(Ask::Permanent { ids, count: p.count, dirs: p.dirs }),
            _ => self.commit_now(ids),
        }
    }

    pub fn commit_now(&mut self, ids: Option<Vec<EntityId>>) {
        self.jobs.start(Long::Commit, move |b, rep| {
            Done::Commit(b.commit_pending(ids, false, move |p| rep.report(if p.total > 0 { p.index as f32 / p.total as f32 } else { 0.0 }, format!("{} / {}  {}", p.index, p.total, p.label))))
        });
    }

    // ---- bin and delete queue ----

    pub fn add_to_bin(&mut self, ids: Vec<EntityId>) {
        let mut n = 0;
        for id in ids {
            if !self.bin.contains(&id) {
                self.bin.push(id);
                n += 1;
            }
        }
        if n > 0 {
            self.info_toast(format!("{n} added to the bin"));
            self.dock_asks.push(DockAsk::Show(Tab::Bin));
        }
        self.refresh_side();
    }

    pub fn queue_delete(&mut self, ids: Vec<EntityId>) {
        let ok: Vec<EntityId> = ids.into_iter().filter(|id| self.row(*id).is_some_and(|r| matches!(r.kind, Kind::Sequence | Kind::File))).collect();
        let n = ok.iter().filter(|id| !self.deletes.contains(id)).count();
        for id in ok {
            if !self.deletes.contains(&id) {
                self.deletes.push(id);
            }
        }
        if n > 0 {
            self.info_toast(format!("{n} queued for delete"));
            self.dock_asks.push(DockAsk::Show(Tab::Deletes));
        }
        self.refresh_side();
    }

    pub fn execute_deletes(&mut self, to_trash: bool) {
        let ids = self.deletes.clone();
        if ids.is_empty() {
            return;
        }
        self.jobs.start(Long::Delete, move |b, rep| {
            Done::Delete(b.delete_entities(ids, to_trash, move |p| rep.report(if p.total > 0 { p.index as f32 / p.total as f32 } else { 0.0 }, format!("deleting {} / {}  {}", p.index, p.total, p.label))))
        });
    }

    // ---- search ----

    pub fn search(&mut self, query: String) {
        let root = self.dir.clone();
        let (r, q) = (root.display().to_string(), query.clone());
        self.search_progress = Some("starting…".into());
        let started = self.jobs.start(Long::Search, move |b, rep| {
            let out = b.search_open(&r, &q, move |p| rep.report(0.0, format!("{} dirs · {} found · {}", p.scanned, p.matches, p.dir)));
            Done::Search { query: q, root: r, out }
        });
        if !started {
            self.search_progress = None;
            self.warn("wait for the running job to finish");
        }
    }

    pub fn exit_search(&mut self) {
        if let Source::Search { .. } = self.source {
            let d = self.dir.clone();
            self.show_source(Source::Dir(d));
        }
    }

    // ---- player ----

    pub fn play(&mut self, id: EntityId) {
        let Some(r) = self.row(id).cloned() else { return };
        if !matches!(r.kind, Kind::Sequence | Kind::File) || r.life == Life::New {
            return;
        }
        self.player.open(&r, self.prefs.color());
        self.dock_asks.push(DockAsk::Show(Tab::Player));
    }

    pub fn send_to_wobble(&mut self, ids: Vec<EntityId>) {
        let samples: Vec<String> = ids
            .iter()
            .filter_map(|id| self.row(*id))
            .filter(|r| r.life == Life::Unchanged && r.broken.is_none() && (r.kind == Kind::Sequence || r.is_movie()))
            .filter_map(|r| self.backend().frame_paths(r.id).first().map(|p| p.display().to_string()))
            .collect();
        if samples.is_empty() {
            return self.warn("send to wobble edit takes unchanged sequences or movies");
        }
        match self.backend().send_to_wobble_edit(samples) {
            Ok(n) => self.info_toast(format!("sent {n} to wobble edit")),
            Err(e) => self.error(e),
        }
    }

    /// Copy `####` paths of rows to the clipboard.
    pub fn copy_paths(&mut self, ctx: &egui::Context, ids: &[EntityId]) {
        let text: Vec<String> = ids.iter().filter_map(|id| self.row(*id)).map(|r| r.hash_path()).collect();
        if text.is_empty() {
            return;
        }
        ctx.copy_text(text.join("\n"));
        self.info_toast(format!("copied {} path(s)", text.len()));
    }

    pub fn copy_nuke(&mut self, ctx: &egui::Context, ids: &[EntityId]) {
        let rows: Vec<&Row> = ids.iter().filter_map(|id| self.row(*id)).collect();
        let (text, n) = crate::nuke::clipboard(&rows, self.prefs.color().input);
        if n == 0 {
            return self.warn("nothing there Nuke can read");
        }
        ctx.copy_text(text);
        self.info_toast(format!("copied {n} element(s) for Nuke: Ctrl+V in Nuke's node graph (with the handler installed)"));
    }

    // ---- between frames ----

    /// Finished jobs come home. True: draw again.
    pub fn poll(&mut self) -> bool {
        let done = self.jobs.poll();
        let any = !done.is_empty();
        for d in done {
            self.finish(d);
        }
        if self.prefs_dirty && !self.keep_prefs {
            self.prefs_dirty = false;
            self.prefs.save();
        }
        any | self.player.poll()
    }

    fn finish(&mut self, d: Done) {
        match d {
            Done::Commit(Ok(o)) => {
                self.meta.clear();
                self.meta_asked.clear();
                self.info.clear();
                if o.committed {
                    self.info_toast(format!("committed: {} operation(s)", o.executed));
                } else if o.cancelled {
                    self.warn(format!("commit cancelled after {} operation(s); edits on those items were dropped", o.executed));
                } else {
                    self.error(format!("commit stopped: {}", o.error.unwrap_or_default()));
                }
                if o.skipped > 0 {
                    self.warn(format!("{} item(s) skipped: their edits no longer apply", o.skipped));
                }
                self.reload();
            }
            Done::Commit(Err(e)) => {
                self.error(e);
                self.reload();
            }
            Done::Search { query, root, out } => {
                self.search_progress = None;
                match out {
                    Ok(v) => {
                        let ids = v.entities.iter().map(|e| e.id).collect();
                        let before = self.source.clone();
                        self.source = Source::Search { root: PathBuf::from(root), query, ids };
                        if !matches!(before, Source::Search { .. }) {
                            self.browser.back.push(before);
                        }
                        self.selection.clear();
                        self.take_view(v);
                    }
                    Err(e) => self.error(e),
                }
            }
            Done::Export { label, out } => match out {
                Ok(true) => self.info_toast(format!("exported {label}")),
                Ok(false) => self.warn(format!("export of {label} cancelled")),
                Err(e) => self.error(format!("export of {label}: {e}")),
            },
            Done::Delete(Ok(o)) => {
                if o.failed.is_empty() {
                    self.info_toast(format!("deleted {} file(s)", o.deleted));
                } else {
                    self.error(format!("deleted {} file(s); {} failed: {}", o.deleted, o.failed.len(), o.failed.first().cloned().unwrap_or_default()));
                }
                self.deletes.clear();
                if self.source == Source::Deletes {
                    let d = self.dir.clone();
                    self.show_source(Source::Dir(d));
                } else {
                    self.reload();
                }
            }
            Done::Delete(Err(e)) => self.error(e),
            Done::Meta(m) => {
                for dto in &m {
                    let meta = Meta { size: dto.total_size, mtime: dto.mtime, owner: dto.owner.clone() };
                    self.meta.insert(dto.id, meta);
                }
                for r in &mut self.rows {
                    if let Some(m) = self.meta.get(&r.id) {
                        r.meta = Some(m.clone());
                    }
                }
                self.gen += 1;
            }
            Done::Info(id, r) => {
                let r = r.map(|m| Info { format: m.format, path: m.path, fields: m.fields.into_iter().map(|f| (f.label, f.value)).collect() });
                self.info.insert(id, r);
            }
        }
    }
}

impl Panels<Tab> for Seq {
    fn begin(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.jobs.set_ctx(&ctx);
        let settled = !ctx.input(|i| i.pointer.any_down());
        if settled && (ctx.zoom_factor() - self.prefs.zoom).abs() > 1e-3 {
            ctx.set_zoom_factor(self.prefs.zoom);
        }
        ctx.set_theme(if self.prefs.light { egui::Theme::Light } else { egui::Theme::Dark });
    }
    fn bar(&mut self, ui: &mut Ui) {
        crate::menus::bar(ui, self);
    }
    fn foot(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("seq.foot").show(ui, |ui| crate::panels::foot(ui, self));
    }
    fn tab(&mut self, ui: &mut Ui, tab: Tab) {
        match tab {
            Tab::Folders => crate::panels::folders(ui, self),
            Tab::Browser => crate::browser::show(ui, self),
            Tab::Info => crate::panels::info(ui, self),
            Tab::Pending => crate::panels::pending(ui, self),
            Tab::Bin => crate::panels::bin(ui, self),
            Tab::Deletes => crate::panels::deletes(ui, self),
            Tab::Player => crate::player::show(ui, self),
        }
    }
    fn end(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        crate::verbs::show(&ctx, self);
        crate::export::show(&ctx, self);
        crate::panels::asks(&ctx, self);
        crate::panels::progress(&ctx, self);
        crate::keys::global(&ctx, self);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.toasts.tick(dt);
        self.toasts.show(&ctx);
        if !self.toasts.is_empty() {
            ctx.request_repaint();
        }
    }
    fn solo_key(&self) -> Option<egui::Key> {
        Some(egui::Key::Q)
    }
    fn tab_bars(&self) -> bool {
        false
    }
    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::Browser | Tab::Player | Tab::Bin | Tab::Deletes => [false, false],
            _ => [false, true],
        }
    }
}
