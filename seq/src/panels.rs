//! The side panels: Places, Info, Pending, the foot, the commit card.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use egui::Ui;
use seq_core::session::Row;
use seq_core::state::EntityId;

use crate::app::Seq;

fn place(ui: &mut Ui, app: &mut Seq, label: &str, p: &Path) {
    let here = app.session.dir.as_deref() == Some(p);
    if ui.selectable_label(here, label).on_hover_text(p.display().to_string()).clicked() {
        app.go(p);
    }
}

pub fn places(ui: &mut Ui, app: &mut Seq) {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    ui.strong("Places");
    place(ui, app, "Home", &home);
    for name in ["Desktop", "Documents", "Downloads", "Movies", "Pictures"] {
        let p = home.join(name);
        if p.is_dir() {
            place(ui, app, name, &p);
        }
    }
    ui.add_space(8.0);
    ui.strong("Drives");
    place(ui, app, "/", Path::new("/"));
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        let mut vols: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("."))).collect();
        vols.sort();
        for v in vols {
            let name = v.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            place(ui, app, &name, &v);
        }
    }
    if !app.recent.is_empty() {
        ui.add_space(8.0);
        ui.strong("Recent");
        for r in app.recent.clone().iter().skip(1) {
            let name = r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.display().to_string());
            place(ui, app, &name, r);
        }
    }
}

fn kv(ui: &mut Ui, k: &str, v: impl Into<String>) {
    ui.weak(k);
    ui.label(v.into());
    ui.end_row();
}

/// One row's staged edits, each with a revert button.
fn edits(ui: &mut Ui, app: &mut Seq, r: &Row) -> Option<(EntityId, Option<usize>)> {
    let mut act = None;
    for (i, t) in r.intents.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button("✕").on_hover_text("revert this edit").clicked() {
                act = Some((r.id, Some(i)));
            }
            ui.label(t);
        });
    }
    if let Some(b) = &r.broken {
        ui.colored_label(ui.visuals().error_fg_color, format!("can't apply: {b}"));
    }
    for c in &r.conflicts {
        ui.colored_label(ui.visuals().warn_fg_color, format!("collides at {}", c.display()));
    }
    let _ = app;
    act
}

fn revert(app: &mut Seq, act: Option<(EntityId, Option<usize>)>) {
    let res = match act {
        Some((id, Some(i))) => app.session.revert(id, i),
        Some((id, None)) => app.session.clear(id),
        None => return,
    };
    if let Err(e) = res {
        app.error(e);
    }
}

pub fn info(ui: &mut Ui, app: &mut Seq) {
    let sel = app.selected();
    let rows: Vec<Row> = sel.iter().filter_map(|id| app.session.row(*id).cloned()).collect();
    let [r] = rows.as_slice() else {
        if rows.is_empty() {
            ui.weak(format!("{} items here", app.session.rows.len()));
        } else {
            ui.strong(format!("{} selected", rows.len()));
            let frames: usize = rows.iter().map(|r| r.frames.len()).sum();
            ui.label(format!("{frames} files on disk"));
        }
        return;
    };
    ui.strong(&r.name);
    ui.add_space(4.0);
    egui::Grid::new("seq.info").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        kv(ui, "type", r.kind.as_str());
        if let Some(s) = &r.seq {
            kv(ui, "frames", format!("{} - {}", s.first, s.last));
            kv(ui, "on disk", s.on_disk.to_string());
            if s.missing > 0 {
                kv(ui, "missing", s.missing.to_string());
            }
            kv(ui, "padding", s.padding.to_string());
        }
        kv(ui, "folder", r.dir.display().to_string());
        if let Some(f) = r.frames.first() {
            if let Ok(m) = std::fs::metadata(f) {
                let total: u64 = if r.frames.len() > 1 { r.frames.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum() } else { m.len() };
                kv(ui, "size", imfram::gallery::files::size_text(total));
            }
        }
    });
    if !r.intents.is_empty() {
        ui.separator();
        ui.strong("Staged");
        ui.label(format!("{} → {}", r.before, r.after));
        let act = edits(ui, app, r);
        revert(app, act);
    }
}

pub fn pending(ui: &mut Ui, app: &mut Seq) {
    let rows = app.session.pending();
    if rows.is_empty() {
        ui.weak("Nothing staged. Edits made in Stage mode wait here until you commit.");
        return;
    }
    ui.horizontal(|ui| {
        let busy = app.running.is_some();
        if ui.add_enabled(!busy, egui::Button::new(format!("Commit {}", rows.len()))).clicked() {
            app.commit(None);
        }
        if ui.add_enabled(!busy, egui::Button::new("Discard all")).clicked() {
            for r in &rows {
                let _ = app.session.clear(r.id);
            }
        }
    });
    ui.separator();
    let mut act = None;
    for r in &rows {
        ui.horizontal(|ui| {
            if ui.small_button("✕").on_hover_text("revert every edit on this item").clicked() {
                act = Some((r.id, None));
            }
            ui.label(format!("{} → {}", r.before, r.after));
        });
        ui.indent(r.id, |ui| {
            ui.weak(r.dir.display().to_string());
            if let Some(a) = edits(ui, app, r) {
                act = Some(a);
            }
        });
        ui.add_space(4.0);
    }
    revert(app, act);
}

pub fn foot(ui: &mut Ui, app: &mut Seq) {
    ui.horizontal(|ui| {
        let n = app.session.rows.len();
        let s = app.browser.selection.len();
        ui.weak(if s > 0 { format!("{n} items · {s} selected") } else { format!("{n} items") });
        let st = app.browser.thumbs.stats();
        if st.queued + st.running > 0 {
            ui.weak(format!("· {} thumbnails loading", st.queued + st.running));
        }
        let p = app.session.registry.pending().len();
        if p > 0 {
            ui.weak(format!("· {p} staged"));
        }
    });
}

pub fn progress(ctx: &egui::Context, app: &mut Seq) {
    let Some(r) = &app.running else { return };
    let p = r.progress.lock().unwrap().clone();
    let cancel = r.cancel.clone();
    imfram::dialog::card(ctx, egui::Id::new("seq.commit"), |ui| {
        ui.heading("Committing");
        let frac = if p.total > 0 { p.index as f32 / p.total as f32 } else { 0.0 };
        ui.add(egui::ProgressBar::new(frac).text(format!("{} / {}", p.index, p.total)));
        ui.weak(&p.label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                cancel.store(true, Ordering::SeqCst);
            }
        });
    });
}
