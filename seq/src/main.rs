//! seq_manip on imfram: a native window over seq_manip's own Rust.
//!
//! `cargo run -p seq`                      open on the last folder (else home)
//! `cargo run -p seq -- --dir PATH`        open on PATH
//! `cargo run -p seq -- --shot DIR`        render once offscreen to DIR/main.png
//! `cargo run -p seq -- --shot DIR --play NAME` the same with NAME playing ("" = first sequence)

mod app;
mod browser;
mod export;
mod jobs;
mod keys;
mod logic;
mod menus;
mod model;
mod nuke;
mod panels;
mod player;
mod prefs;
mod thumbs;
mod verbs;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use imfram::dock::{Docks, Layout, Side, Tree};
use imfram::shell::Gpu;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use app::{DockAsk, Role, Seq, Tab};

struct State {
    gpu: Gpu,
    docks: Docks<Tab>,
    seq: Seq,
}

struct Launcher {
    state: Option<State>,
    shot: Option<PathBuf>,
    play: Option<String>,
    dir: Option<PathBuf>,
}

fn layout_file() -> PathBuf {
    prefs::dir().join("layout.json")
}

fn first_layout() -> Tree<Tab> {
    // Built outward: Browser (Player stacked with it) | Info, Pending, Bin,
    // Delete queue; then Folders on the left.
    let mut t = Tree::new(vec![Tab::Browser, Tab::Player]);
    t.split(Side::Right, 0.74, vec![Tab::Info, Tab::Pending, Tab::Bin, Tab::Deletes]);
    t.split(Side::Left, 0.86, vec![Tab::Folders]);
    t
}

impl Launcher {
    fn open(&mut self, el: &ActiveEventLoop) {
        let gpu = Gpu::standalone().expect("a GPU");
        let mut docks = Docks::new("seq_manip", self.shot.is_some());
        let saved: Option<Layout<Tab, Role>> = if self.shot.is_none() { std::fs::read_to_string(layout_file()).ok().and_then(|s| serde_json::from_str(&s).ok()) } else { None };
        match saved.filter(|l| !l.windows.is_empty()) {
            Some(l) => docks.restore(el, &gpu, l),
            None => {
                docks.add(el, &gpu, Role::Main, first_layout());
            }
        }
        let mut seq = Seq::new(self.dir.clone());
        if self.shot.is_some() {
            seq.ask = None;
            seq.keep_prefs = true;
            if let Some(name) = &self.play {
                let pick = |r: &&model::Row| if name.is_empty() { r.kind == model::Kind::Sequence } else { r.name.contains(name.as_str()) };
                if let Some(id) = seq.rows.iter().find(pick).map(|r| r.id) {
                    seq.play(id);
                }
            } else if let Some(r) = seq.rows.iter().find(|r| r.kind == model::Kind::Sequence) {
                seq.selection = [r.id].into();
            }
        }
        self.state = Some(State { gpu, docks, seq });
    }

    fn shoot(&mut self, dir: &PathBuf) {
        let s = self.state.as_mut().expect("opened");
        std::fs::create_dir_all(dir).expect("shot dir");
        let t = Instant::now();
        loop {
            apply_asks(s);
            s.docks.apply(None, &s.gpu);
            let _ = s.docks.frame(0, &s.gpu, &mut s.seq, Some(vec![]));
            s.seq.poll();
            let st = s.seq.browser.thumbs.stats();
            let busy = st.queued + st.running + st.waiting_upload > 0;
            if (!busy && t.elapsed() > Duration::from_millis(if self.play.is_some() { 2500 } else { 500 })) || t.elapsed() > Duration::from_secs(30) {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let tex = s.docks.frame(0, &s.gpu, &mut s.seq, Some(vec![])).expect("offscreen");
        let path = dir.join("main.png");
        imfram::shell::capture::save_png(&s.gpu, &tex, &path).expect("png");
        eprintln!("wrote {}", path.display());
    }
}

fn apply_asks(s: &mut State) {
    for a in std::mem::take(&mut s.seq.dock_asks) {
        match a {
            DockAsk::Show(t) => s.docks.show(t),
        }
    }
}

fn save_layout(s: &State) {
    let _ = std::fs::create_dir_all(prefs::dir());
    if let Ok(j) = serde_json::to_string_pretty(&s.docks.save()) {
        let _ = std::fs::write(layout_file(), j);
    }
    s.seq.prefs.save();
}

impl ApplicationHandler for Launcher {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_none() {
            self.open(el);
            if let Some(dir) = self.shot.clone() {
                self.shoot(&dir);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        let Some(i) = s.docks.by_window(id) else { return };
        if matches!(event, WindowEvent::CloseRequested) && i == 0 {
            save_layout(s);
            el.exit();
            return;
        }
        if s.docks.event(&s.gpu, i, &event) {
            s.docks.frame(i, &s.gpu, &mut s.seq, None);
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(s) = &mut self.state else { return };
        if s.seq.poll() {
            for h in &mut s.docks.hosts {
                h.win.request_repaint();
            }
        }
        apply_asks(s);
        s.docks.apply(Some(el), &s.gpu);
        s.docks.poll();
        if s.seq.quit || (s.docks.hosts.iter().all(|h| !h.win.is_visible()) && self.shot.is_none()) {
            save_layout(s);
            el.exit();
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(4)));
    }
}

fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let (mut shot, mut dir, mut play) = (None, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--shot" => shot = args.next().map(PathBuf::from),
            "--dir" => dir = args.next().map(PathBuf::from),
            "--play" => play = Some(args.next().filter(|a| !a.starts_with("--")).unwrap_or_default()),
            _ => {}
        }
    }
    let el = EventLoop::new().expect("event loop");
    let mut app = Launcher { state: None, shot, play, dir };
    el.run_app(&mut app).expect("run");
}
