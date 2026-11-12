//! seq_manip on imfram: a native window over seq_manip's own Rust.
//!
//! `cargo run`                      open on the home folder
//! `cargo run -- --dir PATH`        open on PATH
//! `cargo run -- --shot DIR`        render once offscreen to DIR/main.png

mod app;
mod browser;
mod panels;
mod thumbs;
mod verbs;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use imfram::dock::{Docks, Side, Tree};
use imfram::shell::Gpu;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use app::{Role, Seq, Tab};

struct State {
    gpu: Gpu,
    docks: Docks<Tab>,
    seq: Seq,
}

struct Launcher {
    state: Option<State>,
    shot: Option<PathBuf>,
    dir: PathBuf,
}

impl Launcher {
    fn open(&mut self, el: &ActiveEventLoop) {
        let gpu = Gpu::standalone().expect("a GPU");
        let mut docks = Docks::new("seq_manip", self.shot.is_some());
        // Built outward: Browser | Info, Pending 24%, then Places 16% on the left.
        let mut main = Tree::new(vec![Tab::Browser]);
        main.split(Side::Right, 0.76, vec![Tab::Info, Tab::Pending]);
        main.split(Side::Left, 0.84, vec![Tab::Places]);
        docks.add(el, &gpu, Role::Main, main);
        let seq = Seq::new(&self.dir);
        self.state = Some(State { gpu, docks, seq });
    }

    fn shoot(&mut self, dir: &PathBuf) {
        let s = self.state.as_mut().expect("opened");
        std::fs::create_dir_all(dir).expect("shot dir");
        let t = Instant::now();
        loop {
            let _ = s.docks.frame(0, &s.gpu, &mut s.seq, Some(vec![]));
            let st = s.seq.browser.thumbs.stats();
            let busy = st.queued + st.running + st.waiting_upload > 0;
            if (!busy && t.elapsed() > Duration::from_millis(300)) || t.elapsed() > Duration::from_secs(30) {
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

    fn window_event(&mut self, _el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        let Some(i) = s.docks.by_window(id) else { return };
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
        s.docks.apply(Some(el), &s.gpu);
        s.docks.poll();
        if s.docks.hosts.iter().all(|h| !h.win.is_visible()) && self.shot.is_none() {
            el.exit();
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(4)));
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let mut shot = None;
    let mut dir = home();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--shot" => shot = args.next().map(PathBuf::from),
            "--dir" => dir = args.next().map(PathBuf::from).expect("--dir PATH"),
            _ => {}
        }
    }
    let el = EventLoop::new().expect("event loop");
    let mut app = Launcher { state: None, shot, dir };
    el.run_app(&mut app).expect("run");
}
