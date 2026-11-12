//! The folder: a path bar, the verbs, and the items as tiles or a list.
//! A sequence is one item; folders open on double-click.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Key, Modifiers, Ui};
use imfram::gallery::{Details, Gallery, Items, List, Thumbs};
use imfram::table::Column;
use seq_core::session::{Kind, Row};
use seq_core::state::{EntityId, IntentOp, Lifecycle};

use crate::app::Seq;
use crate::thumbs::{self, Src};
use crate::verbs::Verb;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Tiles,
    List,
}

pub struct Browser {
    pub view: View,
    pub gallery: Gallery,
    pub list: List,
    pub thumbs: Thumbs<Src>,
    pub selection: BTreeSet<u64>,
    pub back: Vec<PathBuf>,
    pub fwd: Vec<PathBuf>,
    pub path_edit: String,
    pub filter: String,
    pub show_hidden: bool,
    /// Indices into `session.rows`, filtered and sorted; rebuilt when stale.
    order: Vec<usize>,
    key: (u64, String, bool, Option<(usize, bool)>),
}

fn columns() -> Vec<Column> {
    vec![
        Column::new("Name", 320.0),
        Column::new("Frames", 70.0).align(egui::Align::RIGHT),
        Column::new("Range", 110.0),
        Column::new("Type", 70.0),
        Column::new("Status", 90.0),
    ]
}

impl Browser {
    pub fn new() -> Self {
        let mut gallery = Gallery::new();
        gallery.size = 180.0;
        gallery.metrics.aspect = 16.0 / 9.0;
        let mut list = List::new(columns());
        list.sort = None;
        list.name_badge = false;
        Self {
            view: View::Tiles,
            gallery,
            list,
            thumbs: thumbs::new(),
            selection: BTreeSet::new(),
            back: vec![],
            fwd: vec![],
            path_edit: String::new(),
            filter: String::new(),
            show_hidden: false,
            order: vec![],
            key: (u64::MAX, String::new(), false, None),
        }
    }
}

fn status(r: &Row) -> &'static str {
    if r.broken.is_some() {
        return "broken";
    }
    if !r.conflicts.is_empty() {
        return "conflict";
    }
    match r.lifecycle {
        Lifecycle::Unchanged => "",
        Lifecycle::Modified => "modified",
        Lifecycle::Deleted => "deleted",
        Lifecycle::Departing => "moving out",
        Lifecycle::New => "new copy",
    }
}

fn range(r: &Row) -> String {
    match &r.seq {
        Some(s) => format!("{}-{}", s.first, s.last),
        None => String::new(),
    }
}

fn type_of(r: &Row) -> String {
    match (&r.seq, r.kind) {
        (Some(s), _) => s.extension.to_uppercase(),
        (None, Kind::File) => r.name.rsplit_once('.').map(|(_, e)| e.to_uppercase()).unwrap_or_else(|| "file".into()),
        (None, k) => k.as_str().into(),
    }
}

struct Rows<'a> {
    rows: &'a [Row],
    order: &'a [usize],
}

impl Rows<'_> {
    fn row(&self, i: usize) -> &Row {
        &self.rows[self.order[i]]
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
        Src { frames: self.row(i).frames.clone() }
    }
    fn label(&self, i: usize) -> String {
        let r = self.row(i);
        if r.lifecycle == Lifecycle::Unchanged {
            r.name.clone()
        } else {
            format!("● {}", r.name)
        }
    }
    fn frames(&self, i: usize) -> u32 {
        let r = self.row(i);
        if r.kind == Kind::Sequence {
            self.src(i).count()
        } else {
            1
        }
    }
    fn frame_name(&self, i: usize, f: u32) -> String {
        let src = self.src(i);
        src.at(f).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
    fn badge(&self, i: usize) -> Option<String> {
        let r = self.row(i);
        let st = status(r);
        let rng = r.seq.as_ref().map(|s| if s.missing > 0 { format!("{}-{} ({} missing)", s.first, s.last, s.missing) } else { format!("{}-{}", s.first, s.last) });
        match (rng, st.is_empty()) {
            (Some(r), true) => Some(r),
            (Some(r), false) => Some(format!("{r} · {st}")),
            (None, false) => Some(st.into()),
            (None, true) => None,
        }
    }
    fn kind(&self, i: usize) -> Option<String> {
        Some(type_of(self.row(i)))
    }
    fn has_picture(&self, i: usize) -> bool {
        let r = self.row(i);
        matches!(r.kind, Kind::Sequence | Kind::File) && r.frames.first().is_some_and(|p| seq_core::thumbs::displayable(p))
    }
    fn dim(&self, i: usize) -> bool {
        let r = self.row(i);
        r.hidden || matches!(r.lifecycle, Lifecycle::Deleted | Lifecycle::Departing)
    }
}

impl Details for Rows<'_> {
    fn cell(&self, i: usize, col: usize) -> String {
        let r = self.row(i);
        match col {
            1 => r.seq.as_ref().map(|s| s.on_disk.to_string()).unwrap_or_default(),
            2 => range(r),
            3 => type_of(r),
            4 => status(r).into(),
            _ => String::new(),
        }
    }
}

fn sort_rows(rows: &[Row], order: &mut [usize], sort: Option<(usize, bool)>) {
    let Some((col, up)) = sort else { return };
    let key = |r: &Row| -> (u8, String) {
        let folder_first = if r.kind == Kind::Directory { 0 } else { 1 };
        let k = match col {
            1 => format!("{:012}", r.seq.as_ref().map(|s| s.on_disk).unwrap_or(0)),
            2 => r.seq.as_ref().map(|s| format!("{:012}", s.first as i64 + 1_000_000)).unwrap_or_default(),
            3 => type_of(r),
            4 => status(r).into(),
            _ => r.name.to_lowercase(),
        };
        (folder_first, k)
    };
    order.sort_by(|&a, &b| {
        let o = key(&rows[a]).cmp(&key(&rows[b]));
        if up { o } else { o.reverse() }
    });
}

pub fn show(ui: &mut Ui, app: &mut Seq) {
    bar(ui, app);
    verbs_row(ui, app);
    if let Some((t, err)) = app.note.clone() {
        ui.horizontal(|ui| {
            let c = if err { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
            ui.colored_label(c, t);
            if ui.small_button("✕").clicked() {
                app.note = None;
            }
        });
    }
    let b = &mut app.browser;
    let key = (app.session.gen, b.filter.clone(), b.show_hidden, b.list.sort);
    if key != b.key {
        let f = b.filter.to_lowercase();
        let mut order: Vec<usize> = (0..app.session.rows.len())
            .filter(|&i| {
                let r = &app.session.rows[i];
                (b.show_hidden || !r.hidden) && (f.is_empty() || r.name.to_lowercase().contains(&f))
            })
            .collect();
        sort_rows(&app.session.rows, &mut order, b.list.sort);
        b.order = order;
        b.key = key;
    }
    let items = Rows { rows: &app.session.rows, order: &b.order };
    let ev = match b.view {
        View::Tiles => {
            b.gallery.selection = b.selection.clone();
            let ev = b.gallery.show(ui, &items, &mut b.thumbs);
            b.selection = b.gallery.selection.clone();
            ev
        }
        View::List => {
            b.list.selection = b.selection.clone();
            let ev = b.list.show(ui, &items, &mut b.thumbs);
            b.selection = b.list.selection.clone();
            ev
        }
    };
    if let Some(id) = ev.activated {
        if let Some(r) = app.session.row(id) {
            if r.kind == Kind::Directory && r.lifecycle == Lifecycle::Unchanged {
                let p = r.frames[0].clone();
                app.go(&p);
                return;
            }
        }
    }
    if let Some((_, pos)) = ev.context {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("seq.menu"), pos));
    }
    context_menu(ui, app);
    keys(ui, app);
}

fn bar(ui: &mut Ui, app: &mut Seq) {
    let mut go: Option<PathBuf> = None;
    // 0: a new place, 1: back, 2: forward.
    let mut how = 0;
    let b = &mut app.browser;
    ui.horizontal(|ui| {
        if ui.add_enabled(!b.back.is_empty(), egui::Button::new("⏴")).on_hover_text("back").clicked() {
            go = b.back.last().cloned();
            how = 1;
        }
        if ui.add_enabled(!b.fwd.is_empty(), egui::Button::new("⏵")).on_hover_text("forward").clicked() {
            go = b.fwd.last().cloned();
            how = 2;
        }
        let parent = app.session.dir.as_ref().and_then(|d| d.parent()).map(|p| p.to_path_buf());
        if ui.add_enabled(parent.is_some(), egui::Button::new("⏶")).on_hover_text("up (cmd-up)").clicked() {
            go = parent;
        }
        if ui.button("⟳").on_hover_text("scan again").clicked() {
            go = app.session.dir.clone();
        }
        let right = 330.0;
        let r = ui.add(egui::TextEdit::singleline(&mut b.path_edit).desired_width((ui.available_width() - right).max(80.0)).font(egui::TextStyle::Monospace));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            go = Some(imfram::filebrowser::expand_home(&b.path_edit));
        } else if !r.has_focus() {
            b.path_edit = app.session.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
        }
        ui.add(egui::TextEdit::singleline(&mut b.filter).hint_text("filter").desired_width(90.0));
        ui.toggle_value(&mut b.show_hidden, "hidden").on_hover_text("show dot files");
        ui.selectable_value(&mut b.view, View::Tiles, "tiles");
        ui.selectable_value(&mut b.view, View::List, "list");
        match b.view {
            View::Tiles => ui.add(egui::Slider::new(&mut b.gallery.size, imfram::gallery::MIN_SIZE..=imfram::gallery::MAX_SIZE).show_value(false)).on_hover_text("tile size (cmd-wheel)"),
            View::List => ui.add(egui::Slider::new(&mut b.list.row_h, imfram::gallery::list::MIN_ROW..=imfram::gallery::list::MAX_ROW).show_value(false)).on_hover_text("row size (cmd-wheel)"),
        };
    });
    if let Some(d) = go {
        if Some(&d) == app.session.dir.as_ref() {
            app.refresh();
        } else {
            let (back, fwd) = (app.browser.back.clone(), app.browser.fwd.clone());
            let cur = app.session.dir.clone();
            app.go(&d);
            if app.session.dir.as_ref() != cur.as_ref() && how != 0 {
                // History moves one step instead of growing.
                let (mut back, mut fwd) = (back, fwd);
                let cur = cur.unwrap_or_default();
                if how == 1 {
                    back.pop();
                    fwd.push(cur);
                } else {
                    fwd.pop();
                    back.push(cur);
                }
                app.browser.back = back;
                app.browser.fwd = fwd;
            }
        }
    }
}

/// What the verbs can do with this selection.
pub struct Can {
    pub one: bool,
    pub any: bool,
    pub seqs: bool,
    pub copyable: bool,
}

pub fn can(app: &Seq) -> Can {
    let sel: Vec<&Row> = app.session.rows.iter().filter(|r| app.browser.selection.contains(&r.id)).collect();
    let real = |r: &&Row| r.lifecycle != Lifecycle::New;
    Can {
        one: sel.len() == 1,
        any: !sel.is_empty() && app.running.is_none(),
        seqs: !sel.is_empty() && sel.iter().all(|r| r.kind == Kind::Sequence) && app.running.is_none(),
        copyable: !sel.is_empty() && sel.iter().all(|r| matches!(r.kind, Kind::Sequence | Kind::File)) && sel.iter().all(real) && app.running.is_none(),
    }
}

fn verbs_row(ui: &mut Ui, app: &mut Seq) {
    let c = can(app);
    ui.horizontal(|ui| {
        if ui.add_enabled(c.any, egui::Button::new("Rename")).on_hover_text("F2").clicked() {
            app.verb = Verb::rename(app);
        }
        if ui.add_enabled(c.copyable, egui::Button::new("Duplicate")).on_hover_text("a copy beside it, new name").clicked() {
            app.verb = Verb::duplicate(app);
        }
        if ui.add_enabled(c.copyable, egui::Button::new("Copy to…")).clicked() {
            copy_to(app);
        }
        if ui.add_enabled(c.any, egui::Button::new("Move to…")).clicked() {
            move_to(app);
        }
        ui.separator();
        if ui.add_enabled(c.seqs, egui::Button::new("Renumber")).on_hover_text("offset frame numbers").clicked() {
            app.verb = Verb::offset(app);
        }
        if ui.add_enabled(c.seqs, egui::Button::new("Re-pad")).clicked() {
            app.verb = Verb::repad(app);
        }
        if ui.add_enabled(c.seqs && c.one, egui::Button::new("Trim")).on_hover_text("keep a frame range").clicked() {
            app.verb = Verb::trim(app);
        }
        ui.separator();
        if ui.add_enabled(c.any, egui::Button::new("Delete")).on_hover_text("Delete key (to Trash on commit)").clicked() {
            delete(app);
        }
    });
}

pub fn delete(app: &mut Seq) {
    let edits = app.selected().into_iter().map(|id| (id, IntentOp::Delete)).collect();
    app.apply(edits);
}

pub fn move_to(app: &mut Seq) {
    let start = app.session.dir.clone().unwrap_or_default();
    if let Some(dest) = imfram::filedialog::FileDialog::new().set_directory(&start).pick_folder() {
        let edits = app.selected().into_iter().map(|id| (id, IntentOp::Move { dest: dest.clone() })).collect();
        app.apply(edits);
    }
}

pub fn copy_to(app: &mut Seq) {
    let start = app.session.dir.clone().unwrap_or_default();
    if let Some(dest) = imfram::filedialog::FileDialog::new().set_directory(&start).pick_folder() {
        let mut made = vec![];
        for id in app.selected() {
            match app.session.stage_copy(id, Some(dest.clone()), None, None) {
                Ok(c) => made.push(c),
                Err(e) => {
                    app.error(e);
                    break;
                }
            }
        }
        if app.mode == crate::app::Mode::Instant && !made.is_empty() {
            app.commit(Some(made));
        }
    }
}

fn context_menu(ui: &mut Ui, app: &mut Seq) {
    let id = egui::Id::new("seq.menu");
    let Some(pos) = ui.ctx().data(|d| d.get_temp::<egui::Pos2>(id)) else { return };
    let c = can(app);
    let mut close = false;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(150.0);
            let item = |ui: &mut Ui, on: bool, t: &str| ui.add_enabled(on, egui::Button::new(t).frame(false)).clicked();
            if item(ui, c.any, "Rename…") {
                app.verb = Verb::rename(app);
                close = true;
            }
            if item(ui, c.copyable, "Duplicate…") {
                app.verb = Verb::duplicate(app);
                close = true;
            }
            if item(ui, c.copyable, "Copy to…") {
                close = true;
                copy_to(app);
            }
            if item(ui, c.any, "Move to…") {
                close = true;
                move_to(app);
            }
            ui.separator();
            if item(ui, c.seqs, "Renumber…") {
                app.verb = Verb::offset(app);
                close = true;
            }
            if item(ui, c.seqs, "Re-pad…") {
                app.verb = Verb::repad(app);
                close = true;
            }
            if item(ui, c.seqs && c.one, "Trim…") {
                app.verb = Verb::trim(app);
                close = true;
            }
            ui.separator();
            let staged = app.selected().iter().any(|id| app.session.row(*id).is_some_and(|r| !r.intents.is_empty()));
            if item(ui, staged, "Revert edits") {
                for id in app.selected() {
                    if let Err(e) = app.session.clear(id) {
                        app.error(e);
                    }
                }
                close = true;
            }
            if item(ui, c.one, "Reveal in Finder") {
                if let Some(r) = app.selected().first().and_then(|id| app.session.row(*id)) {
                    let _ = std::process::Command::new("open").arg("-R").arg(&r.frames[0]).spawn();
                }
                close = true;
            }
            if item(ui, c.one, "Copy path") {
                if let Some(r) = app.selected().first().and_then(|id| app.session.row(*id)) {
                    let p = r.frames[0].display().to_string();
                    ui.ctx().copy_text(p);
                }
                close = true;
            }
            ui.separator();
            if item(ui, c.any, "Delete") {
                delete(app);
                close = true;
            }
        });
    });
    let clicked_off = ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if close || ui.input(|i| i.key_pressed(Key::Escape)) || clicked_off {
        ui.ctx().data_mut(|d| d.remove::<egui::Pos2>(id));
    }
}

fn keys(ui: &mut Ui, app: &mut Seq) {
    if app.verb.is_some() || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (del, f2, up, commit) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace) || i.consume_key(Modifiers::COMMAND, Key::Backspace),
            i.consume_key(Modifiers::NONE, Key::F2),
            i.consume_key(Modifiers::COMMAND, Key::ArrowUp),
            i.consume_key(Modifiers::COMMAND, Key::Enter),
        )
    });
    let c = can(app);
    if del && c.any {
        delete(app);
    }
    if f2 && c.any {
        app.verb = Verb::rename(app);
    }
    if up {
        if let Some(p) = app.session.dir.as_ref().and_then(|d| d.parent()).map(|p| p.to_path_buf()) {
            app.go(&p);
        }
    }
    if commit {
        app.commit(None);
    }
}

#[allow(dead_code)]
pub fn ids(rows: &[Row]) -> Vec<EntityId> {
    rows.iter().map(|r| r.id).collect()
}
