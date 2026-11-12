//! The tilt menubar (file, view, colour, prefs) and the right-click menu.

use egui::{Key, Ui};
use imfram::dock::Tab as _;
use imfram::tiltmenu::{Event, Item, Menu};

use crate::app::{Ask, DockAsk, Seq, Tab};
use crate::browser::can;
use crate::prefs::{InputPref, ViewMode, ViewPref};
use crate::verbs::Verb;

fn tab_id(t: Tab) -> &'static str {
    match t {
        Tab::Folders => "tab-folders",
        Tab::Browser => "tab-browser",
        Tab::Info => "tab-info",
        Tab::Pending => "tab-pending",
        Tab::Bin => "tab-bin",
        Tab::Deletes => "tab-deletes",
        Tab::Player => "tab-player",
    }
}

fn menus(app: &Seq) -> Vec<Menu> {
    let mut view = vec![
        Item::choice("view", "browser", vec!["grid".into(), "list".into()], Some(if app.prefs.view == ViewMode::Grid { 0 } else { 1 })),
        Item::check("hidden", "hidden files", app.prefs.shown.contains(&crate::model::Category::Hidden)),
    ];
    for &t in Tab::all() {
        view.push(Item::action(tab_id(t), format!("show {}", t.title())));
    }
    vec![
        Menu {
            id: "file",
            short: "file".into(),
            full: "file".into(),
            items: vec![
                Item::action("open", "open folder…"),
                Item::action("commit", "commit staged edits"),
                Item::action("nuke-handler", "copy Nuke paste handler"),
                Item::action("about", "about seq_manip"),
                Item::action("quit", "quit"),
            ],
        },
        Menu { id: "view", short: "view".into(), full: "view".into(), items: view },
        Menu {
            id: "color",
            short: "colour".into(),
            full: "colour".into(),
            items: vec![
                Item::choice("input", "EXR input space", vec!["linear Rec.709 / sRGB".into(), "ACEScg".into()], Some(if app.prefs.input == InputPref::Linear { 0 } else { 1 })),
                Item::choice(
                    "transform",
                    "view transform",
                    vec!["ACES filmic".into(), "sRGB clip".into(), "raw".into()],
                    Some(match app.prefs.view_transform {
                        ViewPref::Aces => 0,
                        ViewPref::Srgb => 1,
                        ViewPref::Raw => 2,
                    }),
                ),
            ],
        },
        Menu {
            id: "prefs",
            short: "prefs".into(),
            full: "preferences".into(),
            items: vec![
                Item::slider("zoom", "ui zoom", app.prefs.zoom, 0.6, 2.0).step(0.05),
                Item::choice("theme", "theme", vec!["dark".into(), "light".into()], Some(app.prefs.light as usize)),
                Item::check("instant", "Instant mode (edits write at once)", app.prefs.instant),
                Item::action("reset-warnings", "show first-time warnings again"),
            ],
        },
    ]
}

pub fn bar(ui: &mut Ui, app: &mut Seq) {
    let menus = menus(app);
    let mut events = app.bar.bar(ui, &menus);
    events.extend(app.bar.floating_windows(ui.ctx(), &menus));
    for e in events {
        match e {
            Event::Action("quit") => app.quit = true,
            Event::Action("open") => {
                if let Some(d) = imfram::filedialog::FileDialog::new().set_directory(&app.dir).pick_folder() {
                    app.go(&d);
                }
            }
            Event::Action("commit") => app.commit(None),
            Event::Action("nuke-handler") => {
                ui.ctx().copy_text(crate::nuke::handler().to_string());
                app.info_toast("Nuke handler copied: paste it into Nuke's Script Editor and run it");
            }
            Event::Action("about") => app.ask = Some(Ask::Splash),
            Event::Action("reset-warnings") => {
                app.prefs.warned_commit = false;
                app.prefs.warned_delete = false;
                app.prefs.warned_instant = false;
                app.prefs_changed();
            }
            Event::Action(id) => {
                if let Some(&t) = Tab::all().iter().find(|t| tab_id(**t) == id) {
                    app.dock_asks.push(DockAsk::Show(t));
                }
            }
            Event::Pick("view", i) => {
                app.prefs.view = if i == 0 { ViewMode::Grid } else { ViewMode::List };
                app.prefs_changed();
            }
            Event::Toggle("hidden", on) => {
                if on {
                    app.prefs.shown.insert(crate::model::Category::Hidden);
                } else {
                    app.prefs.shown.remove(&crate::model::Category::Hidden);
                }
                app.prefs_changed();
            }
            Event::Pick("input", i) => {
                app.prefs.input = if i == 0 { InputPref::Linear } else { InputPref::AcesCg };
                color_changed(app);
            }
            Event::Pick("transform", i) => {
                app.prefs.view_transform = [ViewPref::Aces, ViewPref::Srgb, ViewPref::Raw][i.min(2)];
                color_changed(app);
            }
            Event::Slide("zoom", v) => {
                app.prefs.zoom = v;
                app.prefs_changed();
            }
            Event::Pick("theme", i) => {
                app.prefs.light = i == 1;
                app.prefs_changed();
            }
            Event::Toggle("instant", on) => {
                if on && !app.prefs.warned_instant {
                    app.ask = Some(Ask::InstantWarn);
                } else {
                    app.prefs.instant = on;
                    app.prefs_changed();
                }
            }
            _ => {}
        }
    }
}

/// New colour decisions: every picture is fetched again (the sources carry
/// the colour, so old textures simply stop matching).
fn color_changed(app: &mut Seq) {
    app.prefs_changed();
    app.gen += 1;
    app.player.set_color(app.prefs.color());
}

/// The right-click menu over the browser's items.
pub fn context(ui: &mut Ui, app: &mut Seq) {
    let id = egui::Id::new("seq.menu");
    let Some(pos) = ui.ctx().data(|d| d.get_temp::<egui::Pos2>(id)) else { return };
    let c = can(app);
    let mut close = false;
    // Kept inside the window.
    let screen = ui.ctx().content_rect();
    let pos = egui::pos2(pos.x.min(screen.right() - 200.0), pos.y.min(screen.bottom() - 380.0).max(screen.top()));
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(180.0);
            let item = |ui: &mut Ui, on: bool, t: &str| ui.add_enabled(on, egui::Button::new(t).frame(false)).clicked();
            if item(ui, c.playable, "▶ Play") {
                if let Some(id) = app.selected_ids().first().copied() {
                    app.play(id);
                }
                close = true;
            }
            if item(ui, c.wobble, "Send to wobble edit") {
                let ids = app.selected_ids();
                app.send_to_wobble(ids);
                close = true;
            }
            ui.separator();
            if item(ui, c.any, "Rename…") {
                app.verb = Verb::rename(app);
                close = true;
            }
            if item(ui, c.seqs, "Offset…") {
                app.verb = Verb::offset(app);
                close = true;
            }
            if item(ui, c.seqs, "Repad…") {
                app.verb = Verb::repad(app);
                close = true;
            }
            if item(ui, c.seqs && c.one, "Trim…") {
                app.verb = Verb::trim(app);
                close = true;
            }
            ui.separator();
            if item(ui, c.copyable, "Duplicate") {
                crate::verbs::duplicate(app);
                close = true;
            }
            if item(ui, c.copyable, "Copy to…") {
                close = true;
                crate::verbs::copy_to(app);
            }
            if item(ui, c.movable, "Move…") {
                close = true;
                crate::verbs::move_to(app);
            }
            if item(ui, c.any, "Delete") {
                crate::verbs::delete(app);
                close = true;
            }
            ui.separator();
            if item(ui, c.binnable, "Add to Bin") {
                let ids = app.selected_ids();
                app.add_to_bin(ids);
                close = true;
            }
            if item(ui, c.copyable, "Queue for delete") {
                let ids = app.selected_ids();
                app.queue_delete(ids);
                close = true;
            }
            ui.separator();
            let staged = app.selected().iter().any(|r| r.pending.is_some());
            if item(ui, staged, "Revert edits") {
                for id in app.selected_ids() {
                    let _ = app.backend().clear_intents(id);
                }
                app.reload();
                close = true;
            }
            if item(ui, c.one, "Reveal in Finder") {
                if let Some(r) = app.selected().first() {
                    let target = app.backend().frame_paths(r.id).first().cloned().unwrap_or(r.path.clone());
                    let _ = std::process::Command::new("open").arg("-R").arg(target).spawn();
                }
                close = true;
            }
            if item(ui, !app.selection.is_empty(), "Copy paths") {
                let ids = app.selected_ids();
                app.copy_paths(ui.ctx(), &ids);
                close = true;
            }
            if item(ui, !app.selection.is_empty(), "Copy for Nuke") {
                let ids = app.selected_ids();
                app.copy_nuke(ui.ctx(), &ids);
                close = true;
            }
            if item(ui, c.exportable, "Export mp4…") {
                app.export = Some(crate::export::Form::new(app.selected_ids()));
                close = true;
            }
        });
    });
    let clicked_off = ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if close || ui.input(|i| i.key_pressed(Key::Escape)) || clicked_off {
        ui.ctx().data_mut(|d| d.remove::<egui::Pos2>(id));
    }
}
