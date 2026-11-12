//! The tabs, the windows, and the app state they share.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

use egui::Ui;
use imfram::dock::{self, Panels};
use seq_core::session::{CommitOutcome, Progress, Session};
use seq_core::state::{EntityId, IntentOp};

use crate::browser::Browser;
use crate::verbs::Verb;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Places,
    Browser,
    Info,
    Pending,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Role {
    Main,
    Detached,
}

impl dock::Tab for Tab {
    type Role = Role;
    fn all() -> &'static [Self] {
        &[Tab::Places, Tab::Browser, Tab::Info, Tab::Pending]
    }
    fn title(self) -> &'static str {
        match self {
            Tab::Places => "Places",
            Tab::Browser => "Browser",
            Tab::Info => "Info",
            Tab::Pending => "Pending",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Tab::Places => "home, common folders, drives, recent",
            Tab::Browser => "the folder, sequences as single items",
            Tab::Info => "the selected item and its staged edits",
            Tab::Pending => "every staged edit, ready to commit",
        }
    }
    fn home(self) -> Role {
        Role::Main
    }
}

impl dock::Role for Role {
    fn title(self) -> &'static str {
        "seq_manip"
    }
    fn size(self) -> (u32, u32) {
        match self {
            Role::Main => (1400, 860),
            Role::Detached => (560, 600),
        }
    }
    fn detached() -> Self {
        Role::Detached
    }
}

/// Stage edits into the tray, or write each one at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Stage,
    Instant,
}

/// A commit running on its own thread.
pub struct Running {
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    rx: Receiver<(Vec<EntityId>, CommitOutcome)>,
}

pub struct Seq {
    pub session: Session,
    pub browser: Browser,
    pub verb: Option<Verb>,
    pub mode: Mode,
    pub running: Option<Running>,
    pub recent: Vec<PathBuf>,
    /// The last error or result, shown until the next.
    pub note: Option<(String, bool)>,
    ctx: Option<egui::Context>,
}

impl Seq {
    pub fn new(dir: &Path) -> Self {
        let mut s = Self {
            session: Session::new(),
            browser: Browser::new(),
            verb: None,
            mode: Mode::Stage,
            running: None,
            recent: vec![],
            note: None,
            ctx: None,
        };
        s.go(dir);
        s
    }

    /// Show a folder.
    pub fn go(&mut self, dir: &Path) {
        let before = self.session.dir.clone();
        match self.session.open(dir) {
            Ok(()) => {
                if let Some(b) = before.filter(|b| Some(b) != self.session.dir.as_ref()) {
                    self.browser.back.push(b);
                    self.browser.fwd.clear();
                }
                if let Some(d) = &self.session.dir {
                    self.recent.retain(|r| r != d);
                    self.recent.insert(0, d.clone());
                    self.recent.truncate(12);
                }
                self.browser.selection.clear();
                self.take_voided();
            }
            Err(e) => self.error(e),
        }
    }

    pub fn refresh(&mut self) {
        if let Err(e) = self.session.refresh() {
            self.error(e);
        }
        self.take_voided();
    }

    fn take_voided(&mut self) {
        if !self.session.voided.is_empty() {
            let names = std::mem::take(&mut self.session.voided).join(", ");
            self.error(format!("staged edits dropped (changed on disk): {names}"));
        }
    }

    pub fn error(&mut self, e: impl Into<String>) {
        self.note = Some((e.into(), true));
    }

    pub fn info(&mut self, t: impl Into<String>) {
        self.note = Some((t.into(), false));
    }

    pub fn selected(&self) -> Vec<EntityId> {
        self.session.rows.iter().filter(|r| self.browser.selection.contains(&r.id)).map(|r| r.id).collect()
    }

    /// Stage one edit on each id; in Instant mode, commit them at once.
    pub fn apply(&mut self, edits: Vec<(EntityId, IntentOp)>) {
        let mut done = vec![];
        for (id, op) in edits {
            match self.session.stage(id, op) {
                Ok(()) => done.push(id),
                Err(e) => {
                    self.error(e);
                    break;
                }
            }
        }
        if self.mode == Mode::Instant && !done.is_empty() {
            self.commit(Some(done));
        }
    }

    /// Start a commit on a thread (all pending, or `only`).
    pub fn commit(&mut self, only: Option<Vec<EntityId>>) {
        if self.running.is_some() {
            return;
        }
        let job = match self.session.begin_commit(only.as_deref()) {
            Ok(j) => j,
            Err(e) => return self.error(e),
        };
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (p, c, ctx) = (progress.clone(), cancel.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let out = job.run(false, &c, |step| {
                *p.lock().unwrap() = step;
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
            });
            let _ = tx.send(out);
            if let Some(ctx) = &ctx {
                ctx.request_repaint();
            }
        });
        self.running = Some(Running { progress, cancel, rx });
    }

    /// Between frames: a finished commit comes home. True: draw again.
    pub fn poll(&mut self) -> bool {
        let Some(r) = &self.running else { return false };
        let Ok((ids, out)) = r.rx.try_recv() else { return false };
        self.running = None;
        self.session.finish_commit(&ids, &out);
        if out.committed {
            self.info(format!("committed {} item(s), {} operation(s)", ids.len(), out.executed));
        } else if out.cancelled {
            self.error(format!("commit cancelled after {} operation(s); staged edits on those items were dropped", out.executed));
        } else {
            self.error(format!("commit failed: {}", out.error.unwrap_or_default()));
        }
        self.browser.selection.retain(|id| self.session.rows.iter().any(|r| r.id == *id));
        true
    }
}

impl Panels<Tab> for Seq {
    fn begin(&mut self, ui: &mut Ui) {
        if self.ctx.is_none() {
            self.ctx = Some(ui.ctx().clone());
        }
    }
    fn bar(&mut self, ui: &mut Ui) {
        ui.selectable_value(&mut self.mode, Mode::Stage, "Stage").on_hover_text("queue edits in Pending; nothing is written until Commit");
        ui.selectable_value(&mut self.mode, Mode::Instant, "Instant").on_hover_text("write each edit to disk at once");
        ui.separator();
        let n = self.session.registry.pending().len();
        let label = if n > 0 { format!("Commit {n}") } else { "Commit".into() };
        if ui.add_enabled(n > 0 && self.running.is_none(), egui::Button::new(label)).on_hover_text("write every staged edit (cmd-enter)").clicked() {
            self.commit(None);
        }
    }
    fn foot(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("seq.foot").show(ui, |ui| crate::panels::foot(ui, self));
    }
    fn tab(&mut self, ui: &mut Ui, tab: Tab) {
        match tab {
            Tab::Places => crate::panels::places(ui, self),
            Tab::Browser => crate::browser::show(ui, self),
            Tab::Info => crate::panels::info(ui, self),
            Tab::Pending => crate::panels::pending(ui, self),
        }
    }
    fn end(&mut self, ui: &mut Ui) {
        crate::verbs::show(ui.ctx(), self);
        crate::panels::progress(ui.ctx(), self);
    }
    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::Browser => [false, false],
            _ => [false, true],
        }
    }
}
