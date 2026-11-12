//! Export mp4: the form (fps, quality), then one encode after another on
//! the backend's ffmpeg. One sequence asks where to save; several are named
//! `<prefix>.mp4` beside their frames, never over an existing file.

use std::path::{Path, PathBuf};

use egui::{Context, Id};
use seq_core::state::EntityId;

use crate::app::Seq;
use crate::jobs::{Done, Long};

pub struct Form {
    pub ids: Vec<EntityId>,
    pub fps: f64,
    pub quality: usize,
}

const QUALITY: [(&str, u32); 3] = [("high", 18), ("medium", 20), ("compact", 23)];

impl Form {
    pub fn new(ids: Vec<EntityId>) -> Self {
        Self { ids, fps: 24.0, quality: 0 }
    }
}

/// A free `<stem>.mp4` (or `<stem>_2.mp4`…) in `dir`.
pub fn free_name(dir: &Path, stem: &str) -> PathBuf {
    let stem = stem.trim_end_matches(['.', '_', '-']);
    let stem = if stem.is_empty() { "export" } else { stem };
    let mut p = dir.join(format!("{stem}.mp4"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}_{n}.mp4"));
        n += 1;
    }
    p
}

pub fn show(ctx: &Context, app: &mut Seq) {
    let Some(mut f) = app.export.take() else { return };
    let mut answer = None;
    imfram::dialog::card(ctx, Id::new("seq.export"), |ui| {
        ui.heading(if f.ids.len() == 1 { "Export mp4".to_string() } else { format!("Export {} sequences to mp4", f.ids.len()) });
        egui::Grid::new("seq.export.g").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("fps");
            ui.add(egui::DragValue::new(&mut f.fps).range(1.0..=120.0));
            ui.end_row();
            ui.label("quality");
            ui.horizontal(|ui| {
                for (i, (name, crf)) in QUALITY.iter().enumerate() {
                    ui.selectable_value(&mut f.quality, i, *name).on_hover_text(format!("H.264 CRF {crf}"));
                }
            });
            ui.end_row();
        });
        if f.ids.len() > 1 {
            ui.weak("each is saved beside its frames as <prefix>.mp4");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Export").clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
    });
    match answer {
        None => app.export = Some(f),
        Some(false) => {}
        Some(true) => start(app, f),
    }
}

fn start(app: &mut Seq, f: Form) {
    let crf = QUALITY[f.quality].1;
    let mut jobs: Vec<(EntityId, PathBuf)> = vec![];
    for id in &f.ids {
        let Some(r) = app.row(*id) else { continue };
        let Some(s) = &r.seq else { continue };
        let dest = if f.ids.len() == 1 {
            let name = format!("{}.mp4", s.prefix.trim_end_matches(['.', '_', '-']));
            match imfram::filedialog::FileDialog::new().set_directory(&r.dir).set_file_name(name).add_filter("mp4", &["mp4"]).save_file() {
                Some(p) => p,
                None => return,
            }
        } else {
            free_name(&r.dir, &s.prefix)
        };
        jobs.push((*id, dest));
    }
    if jobs.is_empty() {
        return;
    }
    let fps = f.fps;
    let single = jobs.len() == 1;
    let started = app.jobs.start(Long::Export, move |b, rep| {
        let n = jobs.len();
        let mut last = Done::Export { label: String::new(), out: Ok(true) };
        let mut done = vec![];
        for (k, (id, dest)) in jobs.into_iter().enumerate() {
            let label = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let r2 = rep.clone();
            let l2 = label.clone();
            let out = b.export_h264(id, fps, crf, &dest.to_string_lossy(), single, move |p| {
                let inner = if p.total > 0 { p.index as f32 / p.total as f32 } else { 0.0 };
                r2.report((k as f32 + inner) / n as f32, format!("{l2}: frame {} / {}  ({}/{n})", p.index, p.total, k + 1));
            });
            let stop = !matches!(out, Ok(true));
            done.push(label.clone());
            last = Done::Export { label: if stop { label } else { done.join(", ") }, out };
            if stop {
                break;
            }
        }
        last
    });
    if !started {
        app.warn("wait for the running job to finish");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_taken_name_gets_a_number() {
        let d = std::env::temp_dir().join(format!("seqexp-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(super::free_name(&d, "shot."), d.join("shot.mp4"));
        std::fs::write(d.join("shot.mp4"), b"").unwrap();
        assert_eq!(super::free_name(&d, "shot_"), d.join("shot_2.mp4"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
