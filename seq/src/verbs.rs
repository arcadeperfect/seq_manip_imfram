//! The verbs that need a few values first: a card over the window, Enter
//! to apply, Escape to cancel.

use egui::{Context, Id, Key, Modifiers, Ui};
use seq_core::session::{Kind, Row};
use seq_core::state::{EntityId, IntentOp, NameComponents};

use crate::app::{Mode, Seq};

pub enum Verb {
    /// One sequence: every part of its name.
    RenameSeq { id: EntityId, prefix: String, delimiter: String, suffix: String, extension: String, padding: usize },
    /// One file or folder: its whole name.
    RenameFile { id: EntityId, name: String },
    /// Several: find and replace in each prefix (sequences) or name (files).
    RenameMany { ids: Vec<EntityId>, find: String, replace: String },
    /// A copy beside the original: new prefix (sequence) or name (file).
    Duplicate { ids: Vec<EntityId>, one: Option<String>, tail: String },
    Offset { ids: Vec<EntityId>, start_at: bool, by: i32, start: i32, first: i32 },
    Repad { ids: Vec<EntityId>, width: usize },
    Trim { id: EntityId, first: i32, last: i32, lo: i32, hi: i32 },
}

fn sel_rows(app: &Seq) -> Vec<Row> {
    app.session.rows.iter().filter(|r| app.browser.selection.contains(&r.id)).cloned().collect()
}

impl Verb {
    pub fn rename(app: &Seq) -> Option<Self> {
        let rows = sel_rows(app);
        match rows.as_slice() {
            [] => None,
            [r] => Some(match &r.seq {
                Some(s) => Verb::RenameSeq {
                    id: r.id,
                    prefix: s.prefix.clone(),
                    delimiter: s.delimiter.clone().unwrap_or_default(),
                    suffix: s.suffix.clone().unwrap_or_default(),
                    extension: s.extension.clone(),
                    padding: s.padding,
                },
                None => Verb::RenameFile { id: r.id, name: r.name.clone() },
            }),
            _ => Some(Verb::RenameMany { ids: rows.iter().map(|r| r.id).collect(), find: String::new(), replace: String::new() }),
        }
    }
    pub fn duplicate(app: &Seq) -> Option<Self> {
        let rows = sel_rows(app);
        let one = match rows.as_slice() {
            [] => return None,
            [r] => Some(match &r.seq {
                Some(s) => format!("{}_copy", s.prefix),
                None => match r.name.rsplit_once('.') {
                    Some((stem, ext)) => format!("{stem}_copy.{ext}"),
                    None => format!("{}_copy", r.name),
                },
            }),
            _ => None,
        };
        Some(Verb::Duplicate { ids: rows.iter().map(|r| r.id).collect(), one, tail: "_copy".into() })
    }
    pub fn offset(app: &Seq) -> Option<Self> {
        let rows = sel_rows(app);
        let first = rows.iter().filter_map(|r| r.seq.as_ref().map(|s| s.first)).min()?;
        Some(Verb::Offset { ids: rows.iter().map(|r| r.id).collect(), start_at: true, by: 0, start: 1001, first })
    }
    pub fn repad(app: &Seq) -> Option<Self> {
        let rows = sel_rows(app);
        let width = rows.iter().filter_map(|r| r.seq.as_ref().map(|s| s.padding)).max()?;
        Some(Verb::Repad { ids: rows.iter().map(|r| r.id).collect(), width })
    }
    pub fn trim(app: &Seq) -> Option<Self> {
        let rows = sel_rows(app);
        let [r] = rows.as_slice() else { return None };
        let s = r.seq.as_ref()?;
        Some(Verb::Trim { id: r.id, first: s.first, last: s.last, lo: s.first, hi: s.last })
    }

    fn title(&self) -> &'static str {
        match self {
            Verb::RenameSeq { .. } | Verb::RenameFile { .. } | Verb::RenameMany { .. } => "Rename",
            Verb::Duplicate { .. } => "Duplicate",
            Verb::Offset { .. } => "Renumber",
            Verb::Repad { .. } => "Re-pad",
            Verb::Trim { .. } => "Trim",
        }
    }

    fn form(&mut self, ui: &mut Ui, app: &Seq) {
        let grid = |ui: &mut Ui, body: &mut dyn FnMut(&mut Ui)| {
            egui::Grid::new("seq.verb").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| body(ui));
        };
        match self {
            Verb::RenameSeq { prefix, delimiter, suffix, extension, padding, .. } => {
                grid(ui, &mut |ui| {
                    ui.label("prefix");
                    { let r = ui.text_edit_singleline(prefix); focus_once(ui, r); }
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
                    ui.label("padding");
                    ui.add(egui::DragValue::new(padding).range(1..=10));
                    ui.end_row();
                });
                let pad = "#".repeat(*padding);
                let ext = if extension.is_empty() { String::new() } else { format!(".{extension}") };
                ui.weak(format!("→ {prefix}{delimiter}{pad}{suffix}{ext}"));
            }
            Verb::RenameFile { name, .. } => {
                { let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)); focus_once(ui, r); }
            }
            Verb::RenameMany { ids, find, replace } => {
                grid(ui, &mut |ui| {
                    ui.label("find");
                    { let r = ui.text_edit_singleline(find); focus_once(ui, r); }
                    ui.end_row();
                    ui.label("replace with");
                    ui.text_edit_singleline(replace);
                    ui.end_row();
                });
                ui.weak(format!("in the prefix of {} items (file names for loose files)", ids.len()));
            }
            Verb::Duplicate { ids, one, tail } => match one {
                Some(name) => {
                    ui.label("new name (prefix for a sequence)");
                    { let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY)); focus_once(ui, r); }
                }
                None => {
                    ui.label(format!("add to the end of each of {} names", ids.len()));
                    { let r = ui.text_edit_singleline(tail); focus_once(ui, r); }
                }
            },
            Verb::Offset { start_at, by, start, first, .. } => {
                ui.horizontal(|ui| {
                    ui.selectable_value(start_at, true, "start at");
                    ui.selectable_value(start_at, false, "shift by");
                });
                if *start_at {
                    ui.add(egui::DragValue::new(start));
                    ui.weak(format!("first frame {first} → {start} (others keep their spacing)"));
                } else {
                    ui.add(egui::DragValue::new(by));
                    ui.weak(format!("first frame {first} → {}", *first + *by));
                }
            }
            Verb::Repad { width, .. } => {
                ui.add(egui::DragValue::new(width).range(1..=10).prefix("digits "));
            }
            Verb::Trim { first, last, lo, hi, .. } => {
                grid(ui, &mut |ui| {
                    ui.label("keep from");
                    ui.add(egui::DragValue::new(first).range(*lo..=*hi));
                    ui.end_row();
                    ui.label("to");
                    ui.add(egui::DragValue::new(last).range(*lo..=*hi));
                    ui.end_row();
                });
                ui.weak("frames outside are deleted on commit");
            }
        }
        let _ = app;
    }

    /// The edits this verb makes, or an error to show.
    fn apply(self, app: &mut Seq) {
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let edits: Vec<(EntityId, IntentOp)> = match self {
            Verb::RenameSeq { id, prefix, delimiter, suffix, extension, padding } => {
                let to = NameComponents { prefix: Some(prefix), delimiter: Some(delimiter), suffix: Some(suffix), extension: Some(extension), padding: Some(padding) };
                vec![(id, IntentOp::Rename { to })]
            }
            Verb::RenameFile { id, name } => vec![(id, IntentOp::RenameFile { name })],
            Verb::RenameMany { ids, find, replace } => {
                if find.is_empty() {
                    return;
                }
                ids.iter()
                    .filter_map(|&id| {
                        let r = app.session.row(id)?;
                        match &r.seq {
                            Some(s) if s.prefix.contains(&find) => Some((id, IntentOp::Rename { to: NameComponents { prefix: Some(s.prefix.replace(&find, &replace)), delimiter: None, suffix: None, extension: None, padding: None } })),
                            None if r.name.contains(&find) => Some((id, IntentOp::RenameFile { name: r.name.replace(&find, &replace) })),
                            _ => None,
                        }
                    })
                    .collect()
            }
            Verb::Duplicate { ids, one, tail } => {
                let mut made = vec![];
                for id in ids {
                    let Some(r) = app.session.row(id).cloned() else { continue };
                    let (prefix, name) = match (&one, r.kind) {
                        (Some(n), Kind::Sequence) => (opt(n), None),
                        (Some(n), _) => (None, opt(n)),
                        (None, Kind::Sequence) => (r.seq.as_ref().map(|s| format!("{}{tail}", s.prefix)), None),
                        (None, _) => (None, Some(match r.name.rsplit_once('.') {
                            Some((stem, ext)) => format!("{stem}{tail}.{ext}"),
                            None => format!("{}{tail}", r.name),
                        })),
                    };
                    match app.session.stage_copy(id, None, prefix, name) {
                        Ok(c) => made.push(c),
                        Err(e) => {
                            app.error(e);
                            break;
                        }
                    }
                }
                if app.mode == Mode::Instant && !made.is_empty() {
                    app.commit(Some(made));
                }
                return;
            }
            Verb::Offset { ids, start_at, by, start, first } => {
                let by = if start_at { start - first } else { by };
                if by == 0 {
                    return;
                }
                ids.into_iter().map(|id| (id, IntentOp::Offset { by, padding: None })).collect()
            }
            Verb::Repad { ids, width } => ids.into_iter().map(|id| (id, IntentOp::Repad { width })).collect(),
            Verb::Trim { id, first, last, .. } => vec![(id, IntentOp::Trim { first: first.min(last), last: first.max(last) })],
        };
        app.apply(edits);
    }
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
        verb.form(ui, app);
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ok = if app.mode == Mode::Instant { "Apply now" } else { "Stage" };
            if ui.button(ok).clicked() {
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
