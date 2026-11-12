//! The verbs: the ones that need values first open a card (Enter applies,
//! Escape cancels); the rest act at once.

use egui::{Context, Id, Key, Modifiers, Ui};
use seq_core::state::{EntityId, IntentOp, NameComponents};

use crate::app::Seq;
use crate::model::{Kind, Row};

pub enum Verb {
    /// One sequence: its name's parts (only changed ones are sent).
    RenameSeq { id: EntityId, was: [String; 4], prefix: String, delimiter: String, suffix: String, extension: String },
    /// One file or folder: its whole name.
    RenameFile { id: EntityId, name: String },
    /// Several: find and replace in each prefix (sequences) or name (files).
    RenameMany { ids: Vec<EntityId>, find: String, replace: String },
    /// One item: copy beside it as a new name (prefix for a sequence).
    CopyAs { id: EntityId, seq: bool, name: String },
    Offset { ids: Vec<EntityId>, by: i32 },
    Repad { ids: Vec<EntityId>, width: usize },
    Trim { id: EntityId, first: i32, last: i32, lo: i32, hi: i32, missing: Vec<i32> },
}

fn sel(app: &Seq) -> Vec<Row> {
    app.selected().into_iter().cloned().collect()
}

impl Verb {
    pub fn rename(app: &Seq) -> Option<Self> {
        let rows = sel(app);
        match rows.as_slice() {
            [] => None,
            [r] => Some(match &r.seq {
                Some(s) => {
                    let parts = [s.prefix.clone(), s.delimiter.clone().unwrap_or_default(), s.suffix.clone().unwrap_or_default(), s.extension.clone()];
                    Verb::RenameSeq { id: r.id, prefix: parts[0].clone(), delimiter: parts[1].clone(), suffix: parts[2].clone(), extension: parts[3].clone(), was: parts }
                }
                None => Verb::RenameFile { id: r.id, name: r.name.clone() },
            }),
            _ => Some(Verb::RenameMany { ids: rows.iter().map(|r| r.id).collect(), find: String::new(), replace: String::new() }),
        }
    }
    pub fn offset(app: &Seq) -> Option<Self> {
        let rows = sel(app);
        rows.iter().any(|r| r.seq.is_some()).then(|| Verb::Offset { ids: rows.iter().map(|r| r.id).collect(), by: 0 })
    }
    pub fn repad(app: &Seq) -> Option<Self> {
        let rows = sel(app);
        let width = rows.iter().filter_map(|r| r.seq.as_ref().map(|s| s.padding)).max()?;
        Some(Verb::Repad { ids: rows.iter().map(|r| r.id).collect(), width })
    }
    pub fn trim(app: &Seq) -> Option<Self> {
        let rows = sel(app);
        let [r] = rows.as_slice() else { return None };
        let s = r.seq.as_ref()?;
        Some(Verb::Trim { id: r.id, first: s.first, last: s.last, lo: s.first, hi: s.last, missing: s.missing_frames.clone() })
    }

    fn title(&self) -> &'static str {
        match self {
            Verb::RenameSeq { .. } | Verb::RenameFile { .. } | Verb::RenameMany { .. } => "Rename",
            Verb::CopyAs { .. } => "Copy as",
            Verb::Offset { .. } => "Offset frames",
            Verb::Repad { .. } => "Repad",
            Verb::Trim { .. } => "Trim",
        }
    }

    fn form(&mut self, ui: &mut Ui) {
        match self {
            Verb::RenameSeq { prefix, delimiter, suffix, extension, .. } => {
                egui::Grid::new("seq.verb").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("prefix");
                    let r = ui.text_edit_singleline(prefix);
                    focus_once(ui, r);
                    ui.end_row();
                    ui.label("delimiter");
                    ui.text_edit_singleline(delimiter);
                    ui.end_row();
                    ui.label("suffix");
                    ui.text_edit_singleline(suffix);
                    ui.end_row();
                    ui.label("extension");
                    ui.text_edit_singleline(extension);
                    ui.end_row();
                });
                ui.weak(format!("→ {prefix}{delimiter}####{suffix}.{extension}"));
            }
            Verb::RenameFile { name, .. } => {
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                focus_once(ui, r);
            }
            Verb::RenameMany { ids, find, replace } => {
                egui::Grid::new("seq.verb").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("find");
                    let r = ui.text_edit_singleline(find);
                    focus_once(ui, r);
                    ui.end_row();
                    ui.label("replace with");
                    ui.text_edit_singleline(replace);
                    ui.end_row();
                });
                ui.weak(format!("in the prefix of each of {} items (the name of loose files)", ids.len()));
            }
            Verb::CopyAs { seq, name, .. } => {
                ui.label(if *seq { "new prefix" } else { "new name" });
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                focus_once(ui, r);
            }
            Verb::Offset { ids, by } => {
                ui.horizontal(|ui| {
                    ui.label("by");
                    let r = ui.add(egui::DragValue::new(by));
                    focus_once(ui, r);
                });
                ui.weak(format!("{} item(s); numbers move, the spacing stays", ids.len()));
            }
            Verb::Repad { width, .. } => {
                ui.horizontal(|ui| {
                    ui.label("digits");
                    ui.add(egui::DragValue::new(width).range(1..=10));
                });
            }
            Verb::Trim { first, last, lo, hi, missing, .. } => {
                egui::Grid::new("seq.verb").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("keep from");
                    ui.add(egui::DragValue::new(first).range(*lo..=*hi));
                    ui.end_row();
                    ui.label("to");
                    ui.add(egui::DragValue::new(last).range(*lo..=*hi));
                    ui.end_row();
                });
                let (a, b) = ((*first).min(*last), (*first).max(*last));
                let on_disk = |x: i32, y: i32| -> i64 { (y - x + 1).max(0) as i64 - missing.iter().filter(|m| **m >= x && **m <= y).count() as i64 };
                let gone = on_disk(*lo, *hi) - on_disk(a, b);
                ui.weak(format!("deletes {gone} frame(s) on commit"));
            }
        }
    }

    fn apply(self, app: &mut Seq) {
        let edits: Vec<(EntityId, IntentOp)> = match self {
            Verb::RenameSeq { id, was, prefix, delimiter, suffix, extension } => {
                let ch = |new: String, old: &String| (new != *old).then_some(new);
                let ext = ch(extension, &was[3]).filter(|e| !e.is_empty());
                let to = NameComponents { prefix: ch(prefix, &was[0]), delimiter: ch(delimiter, &was[1]), suffix: ch(suffix, &was[2]), extension: ext, padding: None };
                if to.prefix.is_none() && to.delimiter.is_none() && to.suffix.is_none() && to.extension.is_none() {
                    return;
                }
                vec![(id, IntentOp::Rename { to })]
            }
            Verb::RenameFile { id, name } => {
                if app.row(id).is_some_and(|r| r.name == name) || name.is_empty() {
                    return;
                }
                vec![(id, IntentOp::RenameFile { name })]
            }
            Verb::RenameMany { ids, find, replace } => {
                if find.is_empty() {
                    return;
                }
                ids.iter()
                    .filter_map(|&id| {
                        let r = app.row(id)?;
                        match &r.seq {
                            Some(s) if s.prefix.contains(&find) => Some((id, IntentOp::Rename { to: NameComponents { prefix: Some(s.prefix.replace(&find, &replace)), delimiter: None, suffix: None, extension: None, padding: None } })),
                            None if r.name.contains(&find) => Some((id, IntentOp::RenameFile { name: r.name.replace(&find, &replace) })),
                            _ => None,
                        }
                    })
                    .collect()
            }
            Verb::CopyAs { id, seq, name } => {
                if name.is_empty() {
                    return;
                }
                let (p, n) = if seq { (Some(name), None) } else { (None, Some(name)) };
                app.copy(vec![(id, None, p, n)]);
                return;
            }
            Verb::Offset { ids, by } => {
                if by == 0 {
                    return;
                }
                ids.into_iter().filter(|id| app.row(*id).is_some_and(|r| r.seq.is_some())).map(|id| (id, IntentOp::Offset { by, padding: None })).collect()
            }
            Verb::Repad { ids, width } => ids.into_iter().filter(|id| app.row(*id).is_some_and(|r| r.seq.is_some())).map(|id| (id, IntentOp::Repad { width })).collect(),
            Verb::Trim { id, first, last, .. } => vec![(id, IntentOp::Trim { first: first.min(last), last: first.max(last) })],
        };
        if !edits.is_empty() {
            app.apply(edits);
        }
    }
}

/// Duplicate: one item asks for its new name; several get `_copy`.
pub fn duplicate(app: &mut Seq) {
    let rows = sel(app);
    match rows.as_slice() {
        [] => {}
        [r] => {
            let (seq, name) = match &r.seq {
                Some(s) => (true, format!("{}_copy", s.prefix)),
                None => (false, copy_name(&r.name)),
            };
            app.verb = Some(Verb::CopyAs { id: r.id, seq, name });
        }
        many => {
            let copies = many
                .iter()
                .map(|r| match &r.seq {
                    Some(s) => (r.id, None, Some(format!("{}_copy", s.prefix)), None),
                    None => (r.id, None, None, Some(copy_name(&r.name))),
                })
                .collect();
            app.copy(copies);
        }
    }
}

fn copy_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}_copy.{ext}"),
        _ => format!("{name}_copy"),
    }
}

pub fn copy_to(app: &mut Seq) {
    let ids: Vec<EntityId> = app.selected().iter().filter(|r| matches!(r.kind, Kind::Sequence | Kind::File)).map(|r| r.id).collect();
    if ids.is_empty() {
        return;
    }
    if let Some(dest) = imfram::filedialog::FileDialog::new().set_directory(&app.dir).pick_folder() {
        let d = dest.display().to_string();
        app.copy(ids.into_iter().map(|id| (id, Some(d.clone()), None, None)).collect());
    }
}

pub fn move_to(app: &mut Seq) {
    let ids = app.selected_ids();
    if ids.is_empty() {
        return;
    }
    if let Some(dest) = imfram::filedialog::FileDialog::new().set_directory(&app.dir).pick_folder() {
        crate::browser::move_ids(app, ids, dest);
    }
}

pub fn delete(app: &mut Seq) {
    let edits = app.selected_ids().into_iter().map(|id| (id, IntentOp::Delete)).collect();
    app.apply(edits);
}

/// Give a field the keyboard the first frame its card is up.
fn focus_once(ui: &Ui, r: egui::Response) {
    let id = Id::new("seq.verb.focused");
    if !ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        r.request_focus();
        ui.ctx().data_mut(|d| d.insert_temp(id, true));
    }
}

pub fn show(ctx: &Context, app: &mut Seq) {
    let Some(mut verb) = app.verb.take() else {
        ctx.data_mut(|d| d.remove::<bool>(Id::new("seq.verb.focused")));
        return;
    };
    let (enter, esc) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
    let mut answer = if enter { Some(true) } else if esc { Some(false) } else { None };
    imfram::dialog::card(ctx, Id::new("seq.verb"), |ui| {
        ui.heading(verb.title());
        ui.add_space(6.0);
        verb.form(ui);
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(if app.prefs.instant { "Apply" } else { "Stage" }).clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    match answer {
        Some(true) => verb.apply(app),
        Some(false) => {}
        None => app.verb = Some(verb),
    }
}
