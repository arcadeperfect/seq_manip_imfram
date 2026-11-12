//! The Browser tab: path bar, toolbar, banners, and the items as tiles or
//! a list. A sequence is one item; a folder opens on click.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use egui::{Key, Modifiers, Ui};
use imfram::gallery::{Details, Dragged, Gallery, GalleryEvent, Items, List, Thumbs};
use imfram::table::Column;
use seq_core::backend::{Backend, Color};
use seq_core::state::{EntityId, IntentOp};

use crate::app::{Seq, Source};
use crate::model::{date_text, human_bytes, in_list, Category, Kind, Life, Row};
use crate::prefs::{Prefs, SortKey, ViewMode};
use crate::thumbs::{self, Src};
use crate::verbs::Verb;

pub struct Browser {
    pub gallery: Gallery,
    pub list: List,
    pub thumbs: Thumbs<Src>,
    pub back: Vec<Source>,
    pub fwd: Vec<Source>,
    pub path_edit: String,
    completion: Option<(Vec<String>, usize)>,
    pub filter: String,
    pub focus_filter: bool,
    /// Indices into `rows`, filtered and sorted; rebuilt when stale.
    pub order: Vec<usize>,
    key: u64,
    typed: (String, Option<Instant>),
}

fn columns() -> Vec<Column> {
    vec![
        Column::new("Name", 300.0),
        Column::new("Frames", 130.0),
        Column::new("Status", 110.0),
        Column::new("Size", 80.0).align(egui::Align::RIGHT),
        Column::new("Modified", 130.0),
        Column::new("Owner", 80.0),
    ]
}

impl Browser {
    pub fn new(backend: Arc<Backend>, prefs: &Prefs) -> Self {
        let mut gallery = Gallery::new();
        gallery.size = prefs.grid_size.clamp(imfram::gallery::MIN_SIZE, imfram::gallery::MAX_SIZE);
        gallery.metrics.aspect = 16.0 / 10.0;
        let mut list = List::new(columns());
        list.row_h = prefs.row_h;
        list.thumb_aspect = 1.5;
        list.name_badge = false;
        list.sort = Some((sort_col(prefs.sort), prefs.ascending));
        Self {
            gallery,
            list,
            thumbs: thumbs::new(backend),
            back: vec![],
            fwd: vec![],
            path_edit: String::new(),
            completion: None,
            filter: String::new(),
            focus_filter: false,
            order: vec![],
            key: u64::MAX,
            typed: (String::new(), None),
        }
    }
    pub fn scrolled_to_top(&mut self) {
        self.gallery.scroll_to(0);
        self.list.scroll_to(0);
    }
}

fn sort_col(k: SortKey) -> usize {
    match k {
        SortKey::Name => 0,
        SortKey::Size => 3,
        SortKey::Modified => 4,
        SortKey::Owner => 5,
    }
}
fn col_sort(c: usize) -> Option<SortKey> {
    match c {
        0 => Some(SortKey::Name),
        3 => Some(SortKey::Size),
        4 => Some(SortKey::Modified),
        5 => Some(SortKey::Owner),
        _ => None,
    }
}

/// The rows as the gallery and list read them.
pub struct Rows<'a> {
    pub rows: &'a [Row],
    pub order: &'a [usize],
    pub color: Color,
    pub image_exts: &'a str,
    pub movie_exts: &'a str,
    /// Show the folder under the name (search, bin, delete review).
    pub show_dir: bool,
}

impl Rows<'_> {
    fn row(&self, i: usize) -> &Row {
        &self.rows[self.order[i]]
    }
    fn pic(&self, i: usize) -> Option<Src> {
        let r = self.row(i);
        let ext = r.ext();
        let ok = in_list(&ext, self.image_exts) || in_list(&ext, self.movie_exts) || seq_core::thumbs::displayable(&r.path);
        Src::of(r, self.color, ok)
    }
}

impl Items for Rows<'_> {
    type Src = Src;
    fn len(&self) -> usize {
        self.order.len()
    }
    fn id(&self, i: usize) -> u64 {
        self.row(i).id
    }
    fn src(&self, i: usize) -> Src {
        self.pic(i).unwrap_or_else(|| Src::of(self.row(i), self.color, true).unwrap_or_else(|| dummy(self.row(i).id, self.color)))
    }
    fn label(&self, i: usize) -> String {
        let r = self.row(i);
        let mut s = r.name.clone();
        if let Some(p) = &r.pending {
            if p.before != r.name && r.life == Life::Modified {
                s = format!("{s}  (was {})", p.before);
            }
        }
        if self.show_dir {
            s = format!("{s}  ·  {}", r.dir.display());
        }
        s
    }
    fn frames(&self, i: usize) -> u32 {
        self.pic(i).map(|s| s.frames).unwrap_or(1)
    }
    fn poster(&self, i: usize) -> u32 {
        self.pic(i).map(|s| s.poster()).unwrap_or(0)
    }
    fn frame_name(&self, i: usize, f: u32) -> String {
        let r = self.row(i);
        match &r.seq {
            Some(s) => crate::logic::nth_frame(s.first, &s.missing_frames, f as usize).to_string(),
            None => format!("{:.0}%", f as f32 / (thumbs::MOVIE_STEPS - 1) as f32 * 96.0),
        }
    }
    fn badge(&self, i: usize) -> Option<String> {
        let r = self.row(i);
        let st = r.status();
        let rng = r.seq.as_ref().map(|s| if s.missing > 0 { format!("{}-{} · {} missing", s.first, s.last, s.missing) } else { format!("{}-{}", s.first, s.last) });
        match (rng, st.is_empty()) {
            (Some(r), true) => Some(r),
            (Some(r), false) => Some(format!("{r} · {st}")),
            (None, false) => Some(st.into()),
            (None, true) => None,
        }
    }
    fn kind(&self, i: usize) -> Option<String> {
        let r = self.row(i);
        Some(match r.kind {
            Kind::Directory => "folder".into(),
            Kind::Symlink => "link".into(),
            Kind::Other => "other".into(),
            _ => {
                let e = r.ext();
                if e.is_empty() {
                    "file".into()
                } else {
                    e.to_uppercase()
                }
            }
        })
    }
    fn has_picture(&self, i: usize) -> bool {
        self.pic(i).is_some()
    }
    fn dim(&self, i: usize) -> bool {
        let r = self.row(i);
        r.hidden || matches!(r.life, Life::Deleted | Life::Departing)
    }
    fn drop_target(&self, i: usize) -> bool {
        let r = self.row(i);
        r.kind == Kind::Directory && r.life == Life::Unchanged
    }
}

fn dummy(id: EntityId, color: Color) -> Src {
    // Never loaded (has_picture is false); just something to hand over.
    Src::of(&Row { id, ..placeholder() }, color, true).expect("a file src")
}

fn placeholder() -> Row {
    Row {
        id: 0,
        kind: Kind::File,
        life: Life::Unchanged,
        hidden: false,
        seq: None,
        name: String::new(),
        path: PathBuf::new(),
        dir: PathBuf::new(),
        pending: None,
        conflicts: vec![],
        broken: None,
        meta: None,
    }
}

impl Details for Rows<'_> {
    fn cell(&self, i: usize, col: usize) -> String {
        let r = self.row(i);
        let m = r.meta.as_ref();
        match col {
            1 => r.seq.as_ref().map(|s| format!("{}–{} ({})", s.first, s.last, s.on_disk)).unwrap_or_default(),
            2 => {
                let mut v = vec![];
                if let Some(s) = &r.seq {
                    v.push(if s.missing > 0 { format!("{} missing", s.missing) } else { "complete".into() });
                }
                if !r.status().is_empty() {
                    v.push(r.status().into());
                }
                v.join(" · ")
            }
            3 => m.and_then(|m| m.size).map(human_bytes).unwrap_or_default(),
            4 => m.and_then(|m| m.mtime).map(date_text).unwrap_or_default(),
            5 => m.and_then(|m| m.owner.clone()).unwrap_or_default(),
            _ => String::new(),
        }
    }
}

pub fn show(ui: &mut Ui, app: &mut Seq) {
    path_bar(ui, app);
    toolbar(ui, app);
    banners(ui, app);
    // Rows in view order.
    let keep = matches!(app.source, Source::Search { .. });
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (app.gen, &app.browser.filter, &app.prefs.shown, &app.prefs.image_exts, &app.prefs.movie_exts, app.prefs.sort as u8, app.prefs.ascending, keep).hash(&mut h);
        h.finish()
    };
    if key != app.browser.key {
        app.browser.order = crate::logic::order(&app.rows, &app.browser.filter, &app.prefs.shown, &app.prefs.image_exts, &app.prefs.movie_exts, app.prefs.sort, app.prefs.ascending, keep);
        app.browser.key = key;
        // A filter that hides a selected row drops it from the selection.
        let shown: std::collections::BTreeSet<u64> = app.browser.order.iter().map(|&i| app.rows[i].id).collect();
        app.selection.retain(|id| shown.contains(id));
    }
    count_line(ui, app);
    if app.prefs.view == ViewMode::List || app.prefs.sort != SortKey::Name {
        app.want_meta();
    }
    let show_dir = !matches!(app.source, Source::Dir(_));
    let b = &mut app.browser;
    let items = Rows { rows: &app.rows, order: &b.order, color: app.prefs.color(), image_exts: &app.prefs.image_exts, movie_exts: &app.prefs.movie_exts, show_dir };
    let before_sort = b.list.sort;
    let ev = match app.prefs.view {
        ViewMode::Grid => {
            b.gallery.selection = app.selection.clone();
            let ev = b.gallery.show(ui, &items, &mut b.thumbs);
            app.selection = b.gallery.selection.clone();
            ev
        }
        ViewMode::List => {
            b.list.selection = app.selection.clone();
            let ev = b.list.show(ui, &items, &mut b.thumbs);
            app.selection = b.list.selection.clone();
            ev
        }
    };
    if (app.browser.gallery.size - app.prefs.grid_size).abs() > 0.5 || (app.browser.list.row_h - app.prefs.row_h).abs() > 0.5 {
        app.prefs.grid_size = app.browser.gallery.size;
        app.prefs.row_h = app.browser.list.row_h;
        app.prefs_changed();
    }
    if app.browser.list.sort != before_sort {
        if let Some((c, up)) = app.browser.list.sort {
            match col_sort(c) {
                Some(k) => {
                    app.prefs.sort = k;
                    app.prefs.ascending = up;
                    app.prefs_changed();
                }
                None => app.browser.list.sort = before_sort,
            }
        }
    }
    events(ui, app, ev);
    typeahead(ui, app);
}

fn events(ui: &mut Ui, app: &mut Seq, ev: GalleryEvent) {
    // Plain click on an unchanged folder goes into it (as the Svelte app).
    if ev.selection_changed && app.selection.len() == 1 && ui.input(|i| i.pointer.primary_released() && i.modifiers.is_none()) {
        let id = *app.selection.iter().next().unwrap();
        if let Some(r) = app.row(id) {
            if r.kind == Kind::Directory && r.life == Life::Unchanged {
                let p = r.path.clone();
                app.go(&p);
                return;
            }
        }
    }
    if let Some(id) = ev.activated.or(ev.preview) {
        open(app, id);
    }
    if let Some((_, pos)) = ev.context {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("seq.menu"), pos));
    }
    if let Some((target, ids)) = ev.dropped {
        drop_onto(app, target, ids);
    }
    crate::menus::context(ui, app);
}

/// Double-click / Enter / Space: a folder opens, media plays.
pub fn open(app: &mut Seq, id: EntityId) {
    let Some(r) = app.row(id).cloned() else { return };
    match r.kind {
        Kind::Directory if r.life == Life::Unchanged => app.go(&r.path),
        Kind::Sequence | Kind::File => app.play(id),
        _ => {}
    }
}

/// Rows dropped onto a row: a folder takes them (a staged move).
pub fn drop_onto(app: &mut Seq, target: Option<u64>, d: Dragged) {
    let Some(t) = target.and_then(|t| app.row(t)).cloned() else { return };
    if t.kind != Kind::Directory || t.life != Life::Unchanged {
        return;
    }
    let ids: Vec<EntityId> = d.0.into_iter().filter(|id| *id != t.id).collect();
    move_ids(app, ids, t.path);
}

pub fn move_ids(app: &mut Seq, ids: Vec<EntityId>, dest: PathBuf) {
    let ids: Vec<EntityId> = ids.into_iter().filter(|id| app.row(*id).is_some_and(|r| !matches!(r.kind, Kind::Symlink | Kind::Other))).collect();
    if ids.is_empty() {
        return;
    }
    app.apply(ids.into_iter().map(|id| (id, IntentOp::Move { dest: dest.clone() })).collect());
}

fn path_bar(ui: &mut Ui, app: &mut Seq) {
    let mut go: Option<PathBuf> = None;
    let mut hist: Option<bool> = None; // Some(true) back, Some(false) forward
    ui.horizontal(|ui| {
        imfram::dock::grip(ui);
        let b = &app.browser;
        if ui.add_enabled(!b.back.is_empty(), egui::Button::new("⏴")).on_hover_text("back").clicked() {
            hist = Some(true);
        }
        if ui.add_enabled(!b.fwd.is_empty(), egui::Button::new("⏵")).on_hover_text("forward").clicked() {
            hist = Some(false);
        }
        let parent = app.dir.parent().map(|p| p.to_path_buf());
        let up = ui.add_enabled(parent.is_some(), egui::Button::new("⏶")).on_hover_text("up (cmd-up) · drop items here to move them up a folder");
        if up.clicked() {
            go = parent.clone();
        }
        if let (Some(d), Some(p)) = (up.dnd_release_payload::<Dragged>(), parent.clone()) {
            move_ids(app, d.0.clone(), p);
        }
        if up.dnd_hover_payload::<Dragged>().is_some() {
            ui.painter().rect_stroke(up.rect.expand(2.0), 3.0, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill), egui::StrokeKind::Outside);
        }
        if ui.button("⟳").on_hover_text("scan again").clicked() {
            app.reload();
        }
        let fav = app.prefs.favourites.contains(&app.dir);
        if ui.button(if fav { "★" } else { "☆" }).on_hover_text(if fav { "remove from favourites" } else { "add this folder to favourites" }).clicked() {
            if fav {
                app.prefs.favourites.retain(|f| f != &app.dir);
            } else {
                app.prefs.favourites.push(app.dir.clone());
            }
            app.prefs_changed();
        }
        let id = egui::Id::new("seq.path");
        let focused = ui.memory(|m| m.has_focus(id));
        // Tab completes (terminal style) instead of moving focus.
        if focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
            complete(app);
        }
        let b = &mut app.browser;
        let w = (ui.available_width() - 40.0).max(80.0);
        let r = ui.add(egui::TextEdit::singleline(&mut b.path_edit).id(id).desired_width(w).font(egui::TextStyle::Monospace).lock_focus(true));
        if r.changed() {
            b.completion = None;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if ui.button("Go").clicked() || enter {
            go = Some(imfram::filebrowser::expand_home(&b.path_edit));
        } else if !r.has_focus() {
            b.path_edit = match &app.source {
                Source::Dir(d) => d.display().to_string(),
                _ => app.dir.display().to_string(),
            };
        }
    });
    if let Some(back) = hist {
        let b = &mut app.browser;
        let target = if back { b.back.pop() } else { b.fwd.pop() };
        if let Some(t) = target {
            let cur = app.source.clone();
            if app.show_source(t.clone()) {
                if back {
                    app.browser.fwd.push(cur);
                } else {
                    app.browser.back.push(cur);
                }
                app.selection.clear();
            } else if back {
                app.browser.back.push(t);
            } else {
                app.browser.fwd.push(t);
            }
        }
    }
    if let Some(d) = go {
        app.go(&d);
    }
}

fn complete(app: &mut Seq) {
    let b = &mut app.browser;
    if let Some((cands, i)) = &mut b.completion {
        *i = (*i + 1) % cands.len();
        b.path_edit = format!("{}/", cands[*i].trim_end_matches('/'));
        return;
    }
    let typed = imfram::filebrowser::expand_home(&b.path_edit).display().to_string();
    let cands = app.jobs.backend.complete_path(&typed);
    match crate::logic::complete(&typed, &cands) {
        Some(t) => b.path_edit = t,
        None if cands.len() > 1 => {
            b.path_edit = format!("{}/", cands[0].trim_end_matches('/'));
            b.completion = Some((cands, 0));
        }
        None => {}
    }
}

/// What the verbs may act on.
pub struct Can {
    pub any: bool,
    pub one: bool,
    pub seqs: bool,
    pub copyable: bool,
    pub movable: bool,
    pub playable: bool,
    pub exportable: bool,
    pub wobble: bool,
    pub binnable: bool,
}

pub fn can(app: &Seq) -> Can {
    let sel = app.selected();
    let free = !app.busy();
    let n = sel.len();
    Can {
        any: n > 0 && free,
        one: n == 1,
        seqs: n > 0 && free && sel.iter().all(|r| r.kind == Kind::Sequence),
        copyable: n > 0 && free && sel.iter().all(|r| matches!(r.kind, Kind::Sequence | Kind::File) && r.life != Life::New),
        movable: n > 0 && free && sel.iter().all(|r| !matches!(r.kind, Kind::Symlink | Kind::Other)),
        playable: n == 1 && sel.iter().all(|r| (r.kind == Kind::Sequence || r.kind == Kind::File) && r.life != Life::New),
        exportable: n > 0 && sel.iter().all(|r| r.kind == Kind::Sequence && r.life == Life::Unchanged && r.broken.is_none()),
        wobble: n > 0 && sel.iter().all(|r| r.life == Life::Unchanged && r.broken.is_none() && (r.kind == Kind::Sequence || r.is_movie())),
        binnable: n > 0 && sel.iter().all(|r| r.life != Life::New),
    }
}

fn toolbar(ui: &mut Ui, app: &mut Seq) {
    let c = can(app);
    ui.horizontal_wrapped(|ui| {
        let mut instant = app.prefs.instant;
        ui.selectable_value(&mut instant, false, "Stage").on_hover_text("queue edits in Pending; nothing is written until Commit");
        ui.selectable_value(&mut instant, true, "Instant").on_hover_text("write each edit to disk at once");
        if ui.small_button("?").on_hover_text("Stage collects edits and writes nothing until you Commit. Instant writes each edit as you make it.").clicked() {
            app.ask = Some(crate::app::Ask::ModeHelp);
        }
        if instant != app.prefs.instant {
            if instant && !app.prefs.warned_instant {
                app.ask = Some(crate::app::Ask::InstantWarn);
            } else {
                app.prefs.instant = instant;
                app.prefs_changed();
            }
        }
        ui.separator();
        if ui.add_enabled(c.playable, egui::Button::new("▶ Play")).on_hover_text("Space / double-click").clicked() {
            if let Some(id) = app.selected_ids().first().copied() {
                app.play(id);
            }
        }
        if ui.add_enabled(c.any, egui::Button::new("Rename")).on_hover_text("F2").clicked() {
            app.verb = Verb::rename(app);
        }
        if ui.add_enabled(c.seqs, egui::Button::new("Offset")).clicked() {
            app.verb = Verb::offset(app);
        }
        if ui.add_enabled(c.seqs, egui::Button::new("Repad")).clicked() {
            app.verb = Verb::repad(app);
        }
        if ui.add_enabled(c.seqs && c.one, egui::Button::new("Trim")).clicked() {
            app.verb = Verb::trim(app);
        }
        ui.separator();
        if ui.add_enabled(c.copyable, egui::Button::new("Duplicate")).clicked() {
            crate::verbs::duplicate(app);
        }
        if ui.add_enabled(c.copyable, egui::Button::new("Copy to…")).clicked() {
            crate::verbs::copy_to(app);
        }
        if ui.add_enabled(c.movable, egui::Button::new("Move…")).clicked() {
            crate::verbs::move_to(app);
        }
        let n = app.selection.len();
        if ui.add_enabled(c.any, egui::Button::new(if n > 0 { format!("Delete ({n})") } else { "Delete".into() })).on_hover_text("Delete / Backspace: staged, to the Trash on commit").clicked() {
            crate::verbs::delete(app);
        }
        if ui.add_enabled(c.binnable, egui::Button::new("To Bin")).on_hover_text("B").clicked() {
            let ids = app.selected_ids();
            app.add_to_bin(ids);
        }
        if ui.add_enabled(c.copyable, egui::Button::new("Queue delete")).on_hover_text("review and delete together, from the Delete queue").clicked() {
            let ids = app.selected_ids();
            app.queue_delete(ids);
        }
        ui.separator();
        if ui.add_enabled(n > 0, egui::Button::new("Copy paths")).clicked() {
            let ids = app.selected_ids();
            app.copy_paths(ui.ctx(), &ids);
        }
        if ui.add_enabled(n > 0, egui::Button::new("Copy for Nuke")).clicked() {
            let ids = app.selected_ids();
            app.copy_nuke(ui.ctx(), &ids);
        }
        if ui.add_enabled(c.exportable, egui::Button::new("Export mp4…")).clicked() {
            app.export = Some(crate::export::Form::new(app.selected_ids()));
        }
    });
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut app.browser.filter).hint_text("filter · Enter scans subfolders").desired_width(220.0).id(egui::Id::new("seq.filter")));
        if app.browser.focus_filter {
            r.request_focus();
            app.browser.focus_filter = false;
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let q = app.browser.filter.clone();
            app.search(q);
        }
        if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            if matches!(app.source, Source::Search { .. }) {
                app.exit_search();
            } else {
                app.browser.filter.clear();
            }
        }
        if ui.button("Scan subfolders").on_hover_text("find sequences and movies by name under this folder").clicked() {
            let q = app.browser.filter.clone();
            app.search(q);
        }
        filters_menu(ui, app);
        ui.separator();
        let mut v = app.prefs.view;
        ui.selectable_value(&mut v, ViewMode::Grid, "grid");
        ui.selectable_value(&mut v, ViewMode::List, "list");
        if v != app.prefs.view {
            app.prefs.view = v;
            app.prefs_changed();
        }
        match app.prefs.view {
            ViewMode::Grid => {
                ui.add(egui::Slider::new(&mut app.browser.gallery.size, imfram::gallery::MIN_SIZE..=imfram::gallery::MAX_SIZE).show_value(false)).on_hover_text("tile size (cmd-wheel)");
                let mut k = app.prefs.sort;
                egui::ComboBox::from_id_salt("seq.sort").selected_text(k.label()).width(90.0).show_ui(ui, |ui| {
                    for s in SortKey::ALL {
                        ui.selectable_value(&mut k, s, s.label());
                    }
                });
                if ui.small_button(if app.prefs.ascending { "⏶" } else { "⏷" }).on_hover_text("sort direction").clicked() {
                    app.prefs.ascending = !app.prefs.ascending;
                    app.browser.list.sort = Some((sort_col(k), app.prefs.ascending));
                    app.prefs_changed();
                }
                if k != app.prefs.sort {
                    app.prefs.sort = k;
                    app.browser.list.sort = Some((sort_col(k), app.prefs.ascending));
                    app.prefs_changed();
                }
            }
            ViewMode::List => {
                ui.add(egui::Slider::new(&mut app.browser.list.row_h, imfram::gallery::list::MIN_ROW..=imfram::gallery::list::MAX_ROW).show_value(false)).on_hover_text("row size (cmd-wheel)");
            }
        }
    });
}

fn filters_menu(ui: &mut Ui, app: &mut Seq) {
    let hidden_n = Category::ALL.len() - app.prefs.shown.len();
    let label = if hidden_n > 0 { format!("Filters ({hidden_n} off)") } else { "Filters".into() };
    ui.menu_button(label, |ui| {
        let mut changed = false;
        for c in Category::ALL {
            let mut on = app.prefs.shown.contains(&c);
            if ui.checkbox(&mut on, c.label()).changed() {
                if on {
                    app.prefs.shown.insert(c);
                } else {
                    app.prefs.shown.remove(&c);
                }
                changed = true;
            }
        }
        ui.separator();
        ui.label("image extensions");
        changed |= ui.text_edit_singleline(&mut app.prefs.image_exts).changed();
        ui.label("movie extensions");
        changed |= ui.text_edit_singleline(&mut app.prefs.movie_exts).changed();
        if ui.button("reset").clicked() {
            let d = Prefs::default();
            app.prefs.shown = d.shown;
            app.prefs.image_exts = d.image_exts;
            app.prefs.movie_exts = d.movie_exts;
            changed = true;
        }
        if changed {
            app.prefs_changed();
        }
    });
}

fn banners(ui: &mut Ui, app: &mut Seq) {
    let warn = ui.visuals().warn_fg_color;
    if let Some(p) = app.search_progress.clone().map(|_| app.jobs.progress()) {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if p.text.is_empty() { "scanning…".to_string() } else { p.text.clone() });
            if ui.button("Cancel").clicked() {
                app.jobs.cancel();
            }
        });
    }
    let mut back_to_dir = false;
    match &app.source {
        Source::Search { root, query, ids } => {
            ui.horizontal(|ui| {
                let q = if query.is_empty() { "everything".to_string() } else { format!("“{query}”") };
                ui.label(format!("{} result(s) for {q} under {}", ids.len(), root.display()));
                if ids.len() >= 200 {
                    ui.colored_label(warn, "(first 200)");
                }
                back_to_dir = ui.button("Exit search").clicked();
            });
        }
        Source::Bin => {
            ui.horizontal(|ui| {
                ui.label(format!("The bin as a view: {} item(s). Every verb works here.", app.rows.len()));
                back_to_dir = ui.button("Back to folder").clicked();
            });
        }
        Source::Deletes => {
            ui.horizontal(|ui| {
                ui.label("Reviewing the delete queue.");
                back_to_dir = ui.button("Back to folder").clicked();
            });
            crate::panels::delete_controls(ui, app);
        }
        Source::Dir(_) => {}
    }
    if back_to_dir {
        let d = app.dir.clone();
        app.show_source(Source::Dir(d));
    }
    if !app.voided.is_empty() {
        ui.horizontal(|ui| {
            ui.colored_label(warn, format!("Staged edits dropped (files changed on disk): {}", app.voided.join(", ")));
            if ui.small_button("×").clicked() {
                app.voided.clear();
            }
        });
    }
}

fn count_line(ui: &mut Ui, app: &Seq) {
    let seqs = app.rows.iter().filter(|r| r.kind == Kind::Sequence).count();
    let items = app.rows.len() - seqs;
    let shown = app.browser.order.len();
    let mut t = format!("{seqs} sequence(s), {items} item(s)");
    if shown != app.rows.len() {
        t.push_str(&format!(" — {shown}/{} match", app.rows.len()));
    }
    if !app.selection.is_empty() {
        t.push_str(&format!(" — {} selected", app.selection.len()));
    }
    ui.weak(t);
}

/// Letters and digits jump to the first name that starts with them (a
/// two-second buffer), while no text field has the keyboard.
fn typeahead(ui: &mut Ui, app: &mut Seq) {
    if ui.ctx().egui_wants_keyboard_input() || app.verb.is_some() || app.ask.is_some() || !ui.ui_contains_pointer() {
        return;
    }
    let typed: String = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) if !i.modifiers.command => Some(t.clone()),
                _ => None,
            })
            .collect()
    });
    // Q shows a panel alone, B adds to the bin, Space plays.
    let typed: String = typed.chars().filter(|c| c.is_alphanumeric() && !matches!(c, 'q' | 'Q' | 'b' | 'B')).collect();
    if typed.is_empty() {
        return;
    }
    let b = &mut app.browser;
    let stale = b.typed.1.is_none_or(|t| t.elapsed().as_secs_f32() > 2.0);
    if stale {
        b.typed.0.clear();
    }
    b.typed.0.push_str(&typed);
    b.typed.1 = Some(Instant::now());
    if let Some(i) = crate::logic::typeahead(&app.rows, &b.order, &b.typed.0) {
        let id = app.rows[b.order[i]].id;
        app.selection = [id].into();
        b.gallery.scroll_to(i);
        b.list.scroll_to(i);
    }
}
