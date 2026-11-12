//! Work off the UI thread: commits, scans, exports, deletes, facts. Each
//! job runs on its own thread against the shared backend, reports progress
//! into a slot the UI reads, and comes home through a channel.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use seq_core::backend::{Backend, CommitOutcome, DeleteOutcome, DirView, MediaInfo, MetaDto};
use seq_core::state::EntityId;

/// What a finished job hands back.
pub enum Done {
    Commit(Result<CommitOutcome, String>),
    Search { query: String, root: String, out: Result<DirView, String> },
    Export { label: String, out: Result<bool, String> },
    Delete(Result<DeleteOutcome, String>),
    Meta(Vec<MetaDto>),
    Info(EntityId, Result<MediaInfo, String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Long {
    Commit,
    Search,
    Export,
    Delete,
}

/// A long job's progress line.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub fraction: f32,
    pub text: String,
}

/// Where a job reports its progress; cheap to clone into callbacks.
#[derive(Clone)]
pub struct Reporter {
    slot: Arc<Mutex<Progress>>,
    ctx: Option<egui::Context>,
}

impl Reporter {
    pub fn report(&self, fraction: f32, text: String) {
        *self.slot.lock().unwrap() = Progress { fraction, text };
        if let Some(c) = &self.ctx {
            c.request_repaint();
        }
    }
}

pub struct Jobs {
    pub backend: Arc<Backend>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    ctx: Option<egui::Context>,
    /// The long job running, if any (one at a time, as the app had it).
    pub long: Option<Long>,
    pub progress: Arc<Mutex<Progress>>,
}

impl Jobs {
    pub fn new(backend: Arc<Backend>) -> Self {
        let (tx, rx) = channel();
        Self { backend, tx, rx, ctx: None, long: None, progress: Arc::default() }
    }

    pub fn set_ctx(&mut self, ctx: &egui::Context) {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
    }

    /// Run `f` on a thread; its result arrives through `poll`.
    pub fn spawn(&self, f: impl FnOnce(&Backend, Reporter) -> Done + Send + 'static) {
        let (b, tx, ctx) = (self.backend.clone(), self.tx.clone(), self.ctx.clone());
        let rep = Reporter { slot: self.progress.clone(), ctx: ctx.clone() };
        std::thread::spawn(move || {
            let done = f(&b, rep);
            let _ = tx.send(done);
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        });
    }

    /// A long job: refused while another runs.
    pub fn start(&mut self, kind: Long, f: impl FnOnce(&Backend, Reporter) -> Done + Send + 'static) -> bool {
        if self.long.is_some() {
            return false;
        }
        self.long = Some(kind);
        *self.progress.lock().unwrap() = Progress::default();
        self.spawn(f);
        true
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    pub fn poll(&mut self) -> Vec<Done> {
        let mut out = vec![];
        while let Ok(d) = self.rx.try_recv() {
            if matches!(d, Done::Commit(_) | Done::Search { .. } | Done::Delete(_) | Done::Export { .. }) {
                self.long = None;
            }
            out.push(d);
        }
        out
    }

    /// Stop the long job.
    pub fn cancel(&self) {
        match self.long {
            Some(Long::Commit) => self.backend.cancel_commit(),
            Some(Long::Search) => self.backend.cancel_search(),
            Some(Long::Export) => self.backend.cancel_export(),
            _ => {}
        }
    }
}
