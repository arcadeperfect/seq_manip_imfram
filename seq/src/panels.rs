//! The side tabs (Folders, Info, Pending, Bin, Delete queue), the foot,
//! and the cards over the window (questions, job progress).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::Ui;
use imfram::gallery::{Dragged, Gallery, List, Thumbs};
use seq_core::backend::Backend;
use seq_core::state::EntityId;

use crate::app::{Ask, Seq, Source, Tab};
use crate::browser::Rows;
use crate::jobs::Long;
use crate::model::{human_bytes, Kind, Life, Row};
use crate::prefs::ViewMode;
use crate::thumbs::{self, Src};

// ---------------------------------------------------------------- folders

/// The folder tree: children read lazily, ancestors of the open folder
/// unfolded.
#[derive(Default)]
pub struct Tree {
    pub open: BTreeSet<PathBuf>,
    kids: HashMap<PathBuf, Vec<(String, PathBuf)>>,
}

impl Tree {
    fn kids(&mut self, b: &Backend, p: &Path) -> &Vec<(String, PathBuf)> {
        self.kids.entry(p.to_path_buf()).or_insert_with(|| b.list_dirs(&p.to_string_lossy()).into_iter().map(|n| (n.name, PathBuf::from(n.path))).collect())
    }
    /// Unfold every ancestor of `dir`.
    pub fn reveal(&mut self, b: &Backend, dir: &Path) {
        for a in dir.ancestors().skip(1) {
            self.open.insert(a.to_path_buf());
            let _ = self.kids(b, a);
        }
        // Refresh the folder itself: it may have new children.
        self.kids.remove(dir);
    }
}

fn node(ui: &mut Ui, app: &mut Seq, name: &str, path: &Path, depth: usize) {
    let open = app.tree.open.contains(path);
    let here = app.dir == path && matches!(app.source, Source::Dir(_));
    let mut toggle = false;
    let mut go = false;
    let r = ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        if ui.add(egui::Button::new(if open { "⏷" } else { "⏵" }).frame(false).small()).clicked() {
            toggle = true;
        }
        let l = ui.selectable_label(here, name);
        if l.clicked() {
            go = true;
        }
        if l.double_clicked() {
            toggle = true;
        }
        l
    });
    let l = r.inner;
    // Drop items on a folder to move them there.
    if let Some(d) = l.dnd_release_payload::<Dragged>() {
        crate::browser::move_ids(app, d.0.clone(), path.to_path_buf());
    }
    if l.dnd_hover_payload::<Dragged>().is_some() {
        ui.painter().rect_stroke(l.rect.expand(1.0), 3.0, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill), egui::StrokeKind::Outside);
    }
    if toggle {
        if open {
            app.tree.open.remove(path);
        } else {
            app.tree.open.insert(path.to_path_buf());
        }
    }
    if go {
        app.go(path);
    }
    if app.tree.open.contains(path) {
        let b = app.jobs.backend.clone();
        let kids = app.tree.kids(&b, path).clone();
        if kids.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space((depth + 2) as f32 * 12.0);
                ui.weak("no folders");
            });
        }
        for (n, p) in kids {
            node(ui, app, &n, &p, depth + 1);
        }
    }
}

pub fn folders(ui: &mut Ui, app: &mut Seq) {
    ui.horizontal(|ui| {
        imfram::dock::grip(ui);
        ui.strong("Favourites");
    });
    if app.prefs.favourites.is_empty() {
        ui.weak("☆ in the path bar adds the open folder");
    }
    let mut drop = None;
    for f in app.prefs.favourites.clone() {
        ui.horizontal(|ui| {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.display().to_string());
            if ui.selectable_label(app.dir == f, format!("★ {name}")).on_hover_text(f.display().to_string()).clicked() {
                app.go(&f);
            }
            if ui.small_button("×").on_hover_text("remove").clicked() {
                drop = Some(f.clone());
            }
        });
    }
    if let Some(f) = drop {
        app.prefs.favourites.retain(|x| x != &f);
        app.prefs_changed();
    }
    ui.add_space(8.0);
    ui.strong("Folders");
    let roots = app.jobs.backend.filesystem_roots();
    for r in roots {
        node(ui, app, &r.name, Path::new(&r.path), 0);
    }
}

// ---------------------------------------------------------------- info

fn kv(ui: &mut Ui, k: &str, v: impl Into<String>) {
    ui.weak(k);
    ui.label(v.into());
    ui.end_row();
}

pub fn info(ui: &mut Ui, app: &mut Seq) {
    ui.horizontal(|ui| imfram::dock::grip(ui));
    let rows: Vec<Row> = app.selected().into_iter().cloned().collect();
    let [r] = rows.as_slice() else {
        if rows.is_empty() {
            ui.weak("Select one item");
        } else {
            ui.weak(format!("{} selected — select one item", rows.len()));
        }
        return;
    };
    ui.label(egui::RichText::new(&r.name).strong());
    if matches!(r.kind, Kind::Sequence | Kind::File) {
        app.want_info(r.id);
    }
    egui::Grid::new("seq.info").num_columns(2).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
        match app.info.get(&r.id) {
            Some(Ok(i)) => {
                kv(ui, "Format", i.format.clone());
                for (k, v) in &i.fields {
                    kv(ui, k, v.clone());
                }
                kv(ui, "Path", i.path.clone());
            }
            Some(Err(e)) => kv(ui, "", e.clone()),
            None => kv(ui, "Folder", r.dir.display().to_string()),
        }
        if let Some(m) = &r.meta {
            if let Some(o) = &m.owner {
                kv(ui, "Owner", o.clone());
            }
            if let Some(t) = m.mtime {
                kv(ui, "Modified", crate::model::date_text(t));
            }
        }
    });
    if let Some(s) = &r.seq {
        if s.missing > 0 {
            let shown: Vec<String> = s.missing_frames.iter().take(200).map(|f| f.to_string()).collect();
            let more = if s.missing as usize > shown.len() { format!(" …and {} more", s.missing as usize - shown.len()) } else { String::new() };
            ui.collapsing(format!("{} missing frame(s)", s.missing), |ui| {
                ui.label(format!("{}{more}", shown.join(", ")));
            });
        }
    }
    if let Some(p) = &r.pending {
        ui.separator();
        ui.strong("Staged");
        ui.label(format!("{} → {}", p.before, p.after));
        let act = intents(ui, r);
        if let Some((id, i)) = act {
            app.revert(id, i);
        }
    }
}

/// A row's staged edits, each with a revert; problems under them.
fn intents(ui: &mut Ui, r: &Row) -> Option<(EntityId, Option<usize>)> {
    let mut act = None;
    if let Some(p) = &r.pending {
        for (i, t) in p.intents.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("×").on_hover_text("revert this step").clicked() {
                    act = Some((r.id, Some(i)));
                }
                ui.label(t);
            });
        }
    }
    if let Some(b) = &r.broken {
        ui.colored_label(ui.visuals().error_fg_color, format!("can't apply: {b}"));
    }
    for c in &r.conflicts {
        ui.colored_label(ui.visuals().warn_fg_color, format!("conflict: collides at {c}"));
    }
    act
}

// ---------------------------------------------------------------- pending

pub fn pending(ui: &mut Ui, app: &mut Seq) {
    let rows = app.pending.clone();
    let conflicts = rows.iter().any(|r| !r.conflicts.is_empty());
    ui.horizontal(|ui| {
        imfram::dock::grip(ui);
        ui.strong(format!("{} staged change(s)", rows.len()));
        let busy = app.jobs.long.is_some();
        if ui.add_enabled(!rows.is_empty() && !busy && !conflicts, egui::Button::new("Commit")).on_hover_text(if conflicts { "resolve the conflicts first" } else { "write every staged edit (cmd-enter)" }).clicked() {
            app.commit(None);
        }
        if ui.add_enabled(!rows.is_empty() && !busy, egui::Button::new("Discard all")).clicked() {
            for r in &rows {
                let _ = app.backend().clear_intents(r.id);
            }
            app.reload();
        }
    });
    if rows.is_empty() {
        ui.weak("Nothing staged. In Stage mode edits wait here until you commit.");
        return;
    }
    ui.separator();
    let mut act = None;
    let mut one = None;
    for r in &rows {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{} → {}", r.before_name(), r.after_name())).strong());
            if !r.conflicts.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color, "conflict").on_hover_text(r.conflicts.join("\n"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("discard").clicked() {
                    act = Some((r.id, None));
                }
                if ui.add_enabled(r.conflicts.is_empty() && r.broken.is_none(), egui::Button::new("commit").small()).clicked() {
                    one = Some(r.id);
                }
            });
        });
        ui.indent(r.id, |ui| {
            ui.weak(r.dir.display().to_string());
            if let Some(a) = intents(ui, r) {
                act = Some(a);
            }
        });
        ui.add_space(4.0);
    }
    if let Some((id, i)) = act {
        app.revert(id, i);
    }
    if let Some(id) = one {
        app.commit(Some(vec![id]));
    }
}

impl Row {
    fn before_name(&self) -> &str {
        self.pending.as_ref().map(|p| p.before.as_str()).unwrap_or(&self.name)
    }
    fn after_name(&self) -> &str {
        self.pending.as_ref().map(|p| p.after.as_str()).unwrap_or(&self.name)
    }
}

// ---------------------------------------------------------------- bin

pub struct BinView {
    pub gallery: Gallery,
    pub list: List,
    pub thumbs: Thumbs<Src>,
    pub selection: BTreeSet<u64>,
}

impl BinView {
    pub fn new(backend: Arc<Backend>) -> Self {
        let mut gallery = Gallery::new();
        gallery.size = 120.0;
        gallery.metrics.aspect = 16.0 / 10.0;
        let mut list = List::new(vec![imfram::table::Column::new("Name", 220.0), imfram::table::Column::new("Frames", 110.0)]);
        list.row_h = 32.0;
        list.name_badge = false;
        Self { gallery, list, thumbs: thumbs::new(backend), selection: BTreeSet::new() }
    }
}

pub fn bin(ui: &mut Ui, app: &mut Seq) {
    let n = app.bin.len();
    let mut ids: Vec<EntityId> = app.bin_view.selection.iter().copied().collect();
    if ids.is_empty() {
        ids = app.bin.clone();
    }
    ui.horizontal_wrapped(|ui| {
        imfram::dock::grip(ui);
        ui.strong(format!("Bin ({n})"));
        if ui.add_enabled(n > 0, egui::Button::new("Open as view")).on_hover_text("list the bin in the browser, where every verb works").clicked() {
            app.show_source(Source::Bin);
            app.selection.clear();
        }
        if ui.add_enabled(n > 0, egui::Button::new("Copy paths")).clicked() {
            app.copy_paths(ui.ctx(), &ids);
        }
        if ui.add_enabled(n > 0, egui::Button::new("Copy for Nuke")).clicked() {
            app.copy_nuke(ui.ctx(), &ids);
        }
        if ui.button("Nuke handler").on_hover_text("copy the Nuke paste handler (run it once in Nuke's Script Editor)").clicked() {
            ui.ctx().copy_text(crate::nuke::handler().to_string());
            app.info_toast("Nuke handler copied: paste it into Nuke's Script Editor and run it");
        }
        if ui.add_enabled(n > 0, egui::Button::new("Clear")).clicked() {
            app.bin.clear();
            app.bin_view.selection.clear();
            app.refresh_side();
        }
        let mut v = app.prefs.bin_view;
        ui.selectable_value(&mut v, ViewMode::List, "list");
        ui.selectable_value(&mut v, ViewMode::Grid, "grid");
        if v != app.prefs.bin_view {
            app.prefs.bin_view = v;
            app.prefs_changed();
        }
    });
    let area = ui.available_rect_before_wrap();
    if n == 0 {
        ui.weak("Drag items here, press B, or use → Bin.");
    } else {
        let rows = app.bin_rows.clone();
        let order: Vec<usize> = (0..rows.len()).collect();
        let items = Rows { rows: &rows, order: &order, color: app.prefs.color(), image_exts: &app.prefs.image_exts, movie_exts: &app.prefs.movie_exts, show_dir: false };
        let bv = &mut app.bin_view;
        let ev = match app.prefs.bin_view {
            ViewMode::Grid => {
                bv.gallery.selection = bv.selection.clone();
                let ev = bv.gallery.show(ui, &items, &mut bv.thumbs);
                bv.selection = bv.gallery.selection.clone();
                ev
            }
            ViewMode::List => {
                bv.list.selection = bv.selection.clone();
                let ev = bv.list.show(ui, &items, &mut bv.thumbs);
                bv.selection = bv.list.selection.clone();
                ev
            }
        };
        // Clicking an item opens its folder with it selected; Enter plays.
        if ev.selection_changed && bv.selection.len() == 1 && ui.input(|i| i.pointer.primary_released() && i.modifiers.is_none()) {
            let id = *bv.selection.iter().next().unwrap();
            if let Some(r) = rows.iter().find(|r| r.id == id) {
                let d = r.dir.clone();
                app.go(&d);
                app.selection = [id].into();
            }
        }
        if let Some(id) = ev.activated.or(ev.preview) {
            app.play(id);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) && ui.ui_contains_pointer() && !ui.ctx().egui_wants_keyboard_input() {
            let gone = app.bin_view.selection.clone();
            app.bin.retain(|id| !gone.contains(id));
            app.bin_view.selection.clear();
            app.refresh_side();
        }
    }
    // Rows dragged in from the browser.
    let resp = ui.interact(area, egui::Id::new("seq.bin.drop"), egui::Sense::hover());
    if let Some(d) = resp.dnd_release_payload::<Dragged>() {
        app.add_to_bin(d.0.clone());
    }
    if resp.dnd_hover_payload::<Dragged>().is_some() {
        ui.painter().rect_stroke(area.shrink(2.0), 4.0, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill), egui::StrokeKind::Inside);
    }
}

// ---------------------------------------------------------------- deletes

pub fn deletes(ui: &mut Ui, app: &mut Seq) {
    let n = app.deletes.len();
    ui.horizontal(|ui| {
        imfram::dock::grip(ui);
        ui.strong(format!("Delete queue ({n})"));
        if ui.add_enabled(n > 0, egui::Button::new("Review & delete")).clicked() {
            app.show_source(Source::Deletes);
            app.selection.clear();
            app.dock_asks.push(crate::app::DockAsk::Show(Tab::Browser));
        }
        if ui.add_enabled(n > 0, egui::Button::new("Clear")).on_hover_text("empty the queue; nothing is deleted").clicked() {
            app.deletes.clear();
            app.refresh_side();
            if app.source == Source::Deletes {
                let d = app.dir.clone();
                app.show_source(Source::Dir(d));
            }
        }
    });
    if n == 0 {
        ui.weak("Queue delete (toolbar or right-click) collects items here to review and delete together.");
        return;
    }
    if let Some((files, bytes, _)) = &app.delete_preview {
        ui.weak(format!("{files} file(s), {}", human_bytes(*bytes)));
    }
    let mut gone = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for r in &app.delete_rows {
            ui.horizontal(|ui| {
                if ui.small_button("×").on_hover_text("take out of the queue").clicked() {
                    gone = Some(r.id);
                }
                ui.label(&r.name).on_hover_text(r.dir.display().to_string());
            });
        }
    });
    if let Some(id) = gone {
        app.deletes.retain(|x| *x != id);
        app.refresh_side();
        if app.source == Source::Deletes {
            app.reload();
        }
    }
}

/// The review page's controls: totals, no-trash folders, trash or not, go.
pub fn delete_controls(ui: &mut Ui, app: &mut Seq) {
    let Some((files, bytes, no_trash)) = app.delete_preview.clone() else { return };
    ui.horizontal(|ui| {
        ui.label(format!("{files} file(s), {}", human_bytes(bytes)));
        ui.checkbox(&mut app.delete_trash, "move to Trash");
        let label = if app.delete_trash { "Move to Trash" } else { "Delete permanently" };
        if ui.add_enabled(app.jobs.long.is_none(), egui::Button::new(label)).clicked() {
            app.ask = Some(Ask::ExecuteDeletes { to_trash: app.delete_trash });
        }
    });
    if !no_trash.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, format!("No Trash on: {} — files there are deleted permanently.", no_trash.join(", ")));
    }
}

// ---------------------------------------------------------------- foot, cards

pub fn foot(ui: &mut Ui, app: &mut Seq) {
    ui.horizontal(|ui| {
        let st = app.browser.thumbs.stats();
        ui.weak(format!("{} item(s)", app.rows.len()));
        if !app.selection.is_empty() {
            ui.weak(format!("· {} selected", app.selection.len()));
        }
        if st.queued + st.running > 0 {
            ui.weak(format!("· {} preview(s) loading", st.queued + st.running));
        }
        if !app.pending.is_empty() {
            ui.weak(format!("· {} staged", app.pending.len()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(if app.prefs.instant { "Instant: edits write at once" } else { "Stage: edits wait for Commit" });
            ui.weak("Q: panel alone");
        });
    });
}

pub fn asks(ctx: &egui::Context, app: &mut Seq) {
    use imfram::dialog::Confirm;
    let Some(ask) = app.ask.take() else { return };
    let answer = match &ask {
        Ask::CommitWarn(_) => Confirm::new("Files will now be changed", "Commit renames, moves, copies and deletes files on disk. Deletes go to the Trash where there is one. Carry on?").yes("Commit").show(ctx),
        Ask::Permanent { count, dirs, .. } => {
            let body = format!("{count} file(s) can't go to a Trash and will be deleted permanently:\n{}", dirs.join("\n"));
            Confirm::new("Permanent deletes", &body).yes("Delete permanently").show(ctx)
        }
        Ask::InstantWarn => Confirm::new("Instant mode", "In Instant mode every edit is written to disk at once, with nothing to review first. Switch?").yes("Switch to Instant").show(ctx),
        Ask::DeleteWarn(_) => Confirm::new("Delete", "Deleted items go to the Trash when you commit (at once in Instant mode). Carry on?").yes("Delete").show(ctx),
        Ask::ExecuteDeletes { to_trash } => {
            let body = if *to_trash { "Move every queued item to the Trash?" } else { "Delete every queued item permanently? This can't be undone." };
            Confirm::new("Delete queue", body).yes(if *to_trash { "Move to Trash" } else { "Delete permanently" }).show(ctx)
        }
        Ask::Splash => {
            let body = "A file browser for image sequences: a numbered frame range is one thing.\n\n• Rename, offset, repad, trim, duplicate, copy, move, delete: applied across every frame.\n• Stage queues edits until you Commit; Instant writes at once.\n• Hover a tile to scrub its frames; Space or double-click plays.\n• Q over any panel shows it alone (again to put it back).\n• The Bin gathers items from anywhere.";
            Confirm::new("seq_manip", body).yes("Got it").no("Close").show(ctx)
        }
        Ask::ModeHelp => {
            let body = "Stage: edits collect in Pending and nothing on disk changes until you Commit. Review, revert single steps, commit all or one.\n\nInstant: each edit is staged and committed at once.";
            Confirm::new("Stage or Instant", body).yes("OK").no("Close").show(ctx)
        }
    };
    match answer {
        None => app.ask = Some(ask),
        Some(yes) => match ask {
            Ask::CommitWarn(ids) => {
                if yes {
                    app.prefs.warned_commit = true;
                    app.prefs_changed();
                    app.commit(ids);
                }
            }
            Ask::Permanent { ids, .. } => {
                if yes {
                    app.commit_now(ids);
                }
            }
            Ask::InstantWarn => {
                app.prefs.warned_instant = true;
                if yes {
                    app.prefs.instant = true;
                }
                app.prefs_changed();
            }
            Ask::DeleteWarn(ids) => {
                if yes {
                    app.prefs.warned_delete = true;
                    app.prefs_changed();
                    app.apply(ids.into_iter().map(|id| (id, seq_core::state::IntentOp::Delete)).collect());
                }
            }
            Ask::ExecuteDeletes { to_trash } => {
                if yes {
                    app.execute_deletes(to_trash);
                }
            }
            Ask::Splash => {
                app.prefs.splash_seen = true;
                app.prefs_changed();
            }
            Ask::ModeHelp => {}
        },
    }
}

/// The running commit / delete / export, as a card with Cancel.
pub fn progress(ctx: &egui::Context, app: &mut Seq) {
    let title = match app.jobs.long {
        Some(Long::Commit) => "Committing",
        Some(Long::Delete) => "Deleting",
        Some(Long::Export) => "Exporting",
        _ => return,
    };
    let p = app.jobs.progress();
    let mut cancel = false;
    imfram::dialog::card(ctx, egui::Id::new("seq.job"), |ui| {
        ui.heading(title);
        ui.add(egui::ProgressBar::new(p.fraction).show_percentage());
        ui.weak(&p.text);
        if app.jobs.long != Some(Long::Delete) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.button("Cancel").clicked();
            });
        }
    });
    if cancel {
        app.jobs.cancel();
    }
}

#[allow(dead_code)]
fn is_media(r: &Row) -> bool {
    matches!(r.kind, Kind::Sequence | Kind::File) && r.life != Life::New
}
