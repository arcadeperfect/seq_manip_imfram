//! The browser laid out for a host's load window (Edify's): the standalone
//! app's dock, flattened into one window the host owns. Down the left the
//! Folders panel (favourites and the folder tree, as the Folders tab);
//! in the middle the browser; down the right the selected item's Info,
//! the staged edits (Pending) and the delete queue, one page at a time.
//! Both sides resize, and their widths and the right side's page are kept
//! in prefs. The Player tab is left out on purpose: a host plays a picked
//! clip in its own viewer.
//!
//! seq_manip's Bin is the TO IMPORT tray, at the foot of the folders side
//! (where wobble keeps its collect-into-bin tray): picks gathered across
//! folders (B, the To Bin verb, or a drag onto it) with their thumbnails
//! and frames, kept while you browse; Backspace or Remove drops the
//! selected ones, ⏶ ⏷ reorder them, Clear empties it, and "Import (n)"
//! tells the host to take them, in the tray's order.

use crate::app::Seq;
use crate::browser::Rows;
use egui::Ui;
use imfram::gallery::Dragged;

/// What the host does after a frame.
#[derive(Default, Debug)]
pub struct Out {
    /// "Import (n)" was pressed: take `tray_rows()` and then `clear_tray`.
    pub import: bool,
}

const PAGES: [&str; 3] = ["Info", "Pending", "Delete queue"];

/// Draw the whole embedded browser into `ui` (fills it).
pub fn show(ui: &mut Ui, app: &mut Seq) -> Out {
    let mut out = Out::default();
    if ui.ui_contains_pointer() {
        keys(ui.ctx(), app);
    }
    let mut tray_h = app.prefs.embed_tray;
    let side = egui::Panel::left(egui::Id::new("seq-embed-folders"))
        .resizable(true)
        .default_size(app.prefs.embed_side)
        .size_range(120.0..=480.0)
        .show(ui, |ui| {
            let t = egui::Panel::bottom(egui::Id::new("seq-embed-tray"))
                .resizable(true)
                .default_size(app.prefs.embed_tray)
                .size_range(90.0..=900.0)
                .show(ui, |ui| out.import = tray(ui, app));
            tray_h = t.response.rect.height();
            egui::ScrollArea::vertical().id_salt("seq-embed-folders-scroll").auto_shrink([false, false]).show(ui, |ui| crate::panels::folders(ui, app));
        });
    let info = egui::Panel::right(egui::Id::new("seq-embed-info"))
        .resizable(true)
        .default_size(app.prefs.embed_info)
        .size_range(160.0..=600.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, name) in PAGES.iter().enumerate() {
                    let label = match i {
                        1 if !app.pending.is_empty() => format!("{name} ({})", app.pending.len()),
                        2 if !app.deletes.is_empty() => format!("{name} ({})", app.deletes.len()),
                        _ => name.to_string(),
                    };
                    if ui.selectable_label(app.prefs.embed_page as usize == i, label).clicked() {
                        app.prefs.embed_page = i as u8;
                        app.prefs_changed();
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("seq-embed-info-scroll").auto_shrink([false, false]).show(ui, |ui| match app.prefs.embed_page {
                1 => crate::panels::pending(ui, app),
                2 => crate::panels::deletes(ui, app),
                _ => crate::panels::info(ui, app),
            });
        });
    egui::CentralPanel::default().show(ui, |ui| crate::browser::show(ui, app));
    // Widths are kept once a resize lets go.
    if !ui.input(|i| i.pointer.any_down()) {
        let (s, w) = (side.response.rect.width(), info.response.rect.width());
        if (s - app.prefs.embed_side).abs() > 0.5 || (w - app.prefs.embed_info).abs() > 0.5 || (tray_h - app.prefs.embed_tray).abs() > 0.5 {
            app.prefs.embed_side = s;
            app.prefs.embed_info = w;
            app.prefs.embed_tray = tray_h;
            app.prefs_changed();
        }
    }
    out
}

/// The standalone window's keys that make sense in a load window, while
/// the pointer is over it: B collects into the tray, cmd+A, cmd+F, cmd+↑,
/// F2 renames, cmd+Enter commits. Not Delete (a load window should not
/// stage deletes from a key; the verb is still there), not Esc (the host
/// closes on it).
fn keys(ctx: &egui::Context, app: &mut Seq) {
    use egui::{Key, Modifiers};
    if app.verb.is_some() || app.ask.is_some() || app.export.is_some() {
        return;
    }
    let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
    if k(Modifiers::COMMAND, Key::F) {
        app.browser.focus_filter = true;
    }
    if k(Modifiers::COMMAND, Key::Enter) {
        app.commit(None);
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    if k(Modifiers::COMMAND, Key::A) {
        app.selection = app.browser.order.iter().map(|&i| app.rows[i].id).collect();
    }
    if k(Modifiers::COMMAND, Key::ArrowUp) {
        if let Some(p) = app.dir.parent().map(|p| p.to_path_buf()) {
            app.go(&p);
        }
    }
    let c = crate::browser::can(app);
    if k(Modifiers::NONE, Key::F2) && c.any {
        app.verb = crate::verbs::Verb::rename(app);
    }
    if k(Modifiers::NONE, Key::B) && c.binnable {
        let ids = app.selected_ids();
        app.add_to_bin(ids);
    }
}

/// The tray's rows, in its order.
pub fn tray_rows(app: &Seq) -> Vec<crate::model::Row> {
    app.bin.iter().filter_map(|id| app.bin_rows.iter().find(|r| r.id == *id).cloned()).collect()
}

/// Empty the tray (after the host took it).
pub fn clear_tray(app: &mut Seq) {
    app.bin.clear();
    app.bin_view.selection.clear();
    app.refresh_side();
}

/// Put `ids` in the tray (B / To Bin do the same).
pub fn add_to_tray(app: &mut Seq, ids: Vec<u64>) {
    app.add_to_bin(ids);
}

fn tray(ui: &mut Ui, app: &mut Seq) -> bool {
    let rows = tray_rows(app);
    let n = rows.len();
    let mut import = false;
    let sel: Vec<u64> = app.bin_view.selection.iter().copied().filter(|id| app.bin.contains(id)).collect();
    ui.horizontal_wrapped(|ui| {
        ui.strong(format!("To import ({n})"));
        import = ui.add_enabled(n > 0, egui::Button::new(format!("Import ({n})"))).on_hover_text("into the bin, in this order").clicked();
        if ui.add_enabled(!sel.is_empty(), egui::Button::new("⏶").small()).on_hover_text("move the selected up").clicked() {
            shift(&mut app.bin, &sel, -1);
        }
        if ui.add_enabled(!sel.is_empty(), egui::Button::new("⏷").small()).on_hover_text("move the selected down").clicked() {
            shift(&mut app.bin, &sel, 1);
        }
        if ui.add_enabled(!sel.is_empty(), egui::Button::new("Remove").small()).on_hover_text("the selected (Backspace)").clicked() {
            remove(app, &sel);
        }
        if ui.add_enabled(n > 0, egui::Button::new("Clear").small()).clicked() {
            clear_tray(app);
        }
    });
    let area = ui.available_rect_before_wrap();
    if n == 0 {
        ui.weak("B, To Bin or a drag here collects clips from any folder.");
    } else {
        let order: Vec<usize> = (0..rows.len()).collect();
        let items = Rows { rows: &rows, order: &order, color: app.prefs.color(), image_exts: &app.prefs.image_exts, movie_exts: &app.prefs.movie_exts, show_dir: true };
        let bv = &mut app.bin_view;
        bv.list.selection = bv.selection.clone();
        let ev = bv.list.show(ui, &items, &mut bv.thumbs);
        bv.selection = bv.list.selection.clone();
        // A click shows the item in its folder.
        if let Some(id) = ev.activated {
            if let Some(r) = rows.iter().find(|r| r.id == id) {
                let d = r.dir.clone();
                app.go(&d);
                app.selection = [id].into();
            }
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing && ui.ui_contains_pointer() && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            let gone: Vec<u64> = app.bin_view.selection.iter().copied().collect();
            remove(app, &gone);
        }
    }
    let resp = ui.interact(area, egui::Id::new("seq-embed-tray-drop"), egui::Sense::hover());
    if let Some(d) = resp.dnd_release_payload::<Dragged>() {
        let fresh: Vec<u64> = d.0.iter().copied().filter(|id| !app.bin.contains(id)).collect();
        if !fresh.is_empty() {
            app.add_to_bin(fresh);
        }
    }
    if resp.dnd_hover_payload::<Dragged>().is_some() {
        ui.painter().rect_stroke(area.shrink(2.0), 4.0, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill), egui::StrokeKind::Inside);
    }
    import
}

fn remove(app: &mut Seq, ids: &[u64]) {
    app.bin.retain(|id| !ids.contains(id));
    app.bin_view.selection.clear();
    app.refresh_side();
}

/// Move the items `sel` one place up (-1) or down (+1), as a block.
fn shift(v: &mut [u64], sel: &[u64], dir: i32) {
    if dir < 0 {
        for i in 1..v.len() {
            if sel.contains(&v[i]) && !sel.contains(&v[i - 1]) {
                v.swap(i, i - 1);
            }
        }
    } else {
        for i in (0..v.len().saturating_sub(1)).rev() {
            if sel.contains(&v[i]) && !sel.contains(&v[i + 1]) {
                v.swap(i, i + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::shift;

    #[test]
    fn the_tray_reorders_a_selection_as_a_block() {
        let mut v = vec![1, 2, 3, 4];
        shift(&mut v, &[3, 4], -1);
        assert_eq!(v, [1, 3, 4, 2]);
        shift(&mut v, &[1], 1);
        assert_eq!(v, [3, 1, 4, 2]);
        shift(&mut v, &[3], -1);
        assert_eq!(v, [3, 1, 4, 2]);
    }
}
