//! The window's keys (the Svelte app's main-window bindings), off while a
//! text field, a card or the player has the keyboard.

use egui::{Context, Key, Modifiers};

use crate::app::{DockAsk, Seq, Tab};
use crate::browser::can;
use crate::verbs::Verb;

pub fn global(ctx: &Context, app: &mut Seq) {
    if app.verb.is_some() || app.ask.is_some() || app.export.is_some() || app.player.has_keys {
        return;
    }
    let typing = ctx.egui_wants_keyboard_input();
    let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
    if k(Modifiers::COMMAND, Key::F) {
        app.browser.focus_filter = true;
        app.dock_asks.push(DockAsk::Show(Tab::Browser));
    }
    if k(Modifiers::COMMAND, Key::I) {
        app.dock_asks.push(DockAsk::Show(Tab::Info));
    }
    if k(Modifiers::COMMAND, Key::Enter) {
        app.commit(None);
    }
    if typing {
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
    let c = can(app);
    if k(Modifiers::NONE, Key::F2) && c.any {
        app.verb = Verb::rename(app);
    }
    if (k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace)) && c.any {
        crate::verbs::delete(app);
    }
    if k(Modifiers::COMMAND, Key::Backspace) && c.copyable {
        let ids = app.selected_ids();
        app.queue_delete(ids);
    }
    if k(Modifiers::NONE, Key::B) && c.binnable {
        let ids = app.selected_ids();
        app.add_to_bin(ids);
    }
    if k(Modifiers::NONE, Key::Escape) {
        if ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::new("seq.menu"))).is_some() {
            ctx.data_mut(|d| d.remove::<egui::Pos2>(egui::Id::new("seq.menu")));
        } else if matches!(app.source, crate::app::Source::Search { .. }) && app.selection.is_empty() {
            app.exit_search();
        } else {
            app.selection.clear();
        }
    }
}
