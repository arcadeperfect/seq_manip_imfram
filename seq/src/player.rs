//! The Player tab: a sequence or movie at full size, with the Svelte
//! player's transport and hands.
//!
//! Frames come as JPEG bytes (a sequence's from the app's `playback_frame`,
//! disk-cached; a movie's from one ffmpeg run that streams them), are kept
//! in RAM for the whole clip, and are decoded to textures only round the
//! playhead, so memory stays bounded. Eight workers fetch outward from the
//! playhead in the play direction; an unbuffered frame shows its nearest
//! buffered neighbour, and playback holds rather than skipping one.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use egui::{pos2, vec2, Color32, ColorImage, Key, Modifiers, PointerButton, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2};
use seq_core::backend::{Backend, Color};
use seq_core::state::EntityId;
use seq_core::thumbs::ViewTransform;

use crate::app::Seq;
use crate::model::{Kind, Row};

const WORKERS: usize = 8;
/// Fetches queued or running at once.
const IN_FLIGHT: usize = 12;
/// Textures kept round the playhead: behind, ahead (in the play direction).
const BEHIND: usize = 8;
const AHEAD: usize = 32;
const MAX_DIM: u32 = 2048;
const MOVIE_DIM: u32 = 1920;
const FPS: [f32; 6] = [12.0, 24.0, 25.0, 30.0, 48.0, 60.0];
const SPEEDS: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

#[derive(Clone)]
enum Task {
    /// A sequence frame: fetch its bytes (and decode it when `decode`).
    Fetch { gen: u64, id: EntityId, i: usize, color: Color, exposure: f32, decode: bool },
    Decode { gen: u64, i: usize, bytes: Arc<Vec<u8>> },
}

enum Msg {
    Bytes { gen: u64, i: usize, out: Result<Arc<Vec<u8>>, String> },
    Image { gen: u64, i: usize, img: ColorImage },
    MovieMeta { gen: u64, fps: f32, total: usize },
    MovieFrame { gen: u64, bytes: Arc<Vec<u8>> },
    MovieEnd { gen: u64, err: Option<String> },
}

/// The workers' queue.
#[derive(Default)]
struct Queue {
    tasks: Mutex<VecDeque<Task>>,
    ready: Condvar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum What {
    Sequence,
    Movie,
}

struct Media {
    id: EntityId,
    name: String,
    what: What,
    /// Sequences: the span's first frame and holes, to name frames.
    first: i32,
    missing: Vec<i32>,
    total: usize,
    bytes: Vec<Option<Arc<Vec<u8>>>>,
    failed: HashSet<usize>,
    fetching: HashSet<usize>,
    decoding: HashSet<usize>,
    tex: HashMap<usize, (TextureHandle, [usize; 2])>,
    fps_i: usize,
    speed_i: usize,
    movie_fps: f32,
    movie_done: bool,
    movie_err: Option<String>,
    cancel: Arc<AtomicBool>,
}

impl Media {
    fn fps(&self) -> f32 {
        match self.what {
            What::Sequence => FPS[self.fps_i],
            What::Movie => self.movie_fps * SPEEDS[self.speed_i],
        }
    }
    fn loaded(&self) -> usize {
        self.bytes.iter().filter(|b| b.is_some()).count()
    }
    fn frame_name(&self, i: usize) -> String {
        match self.what {
            What::Sequence => crate::logic::nth_frame(self.first, &self.missing, i).to_string(),
            What::Movie => {
                let t = i as f32 / self.movie_fps.max(1.0);
                format!("{}:{:05.2}", (t / 60.0) as u32, t % 60.0)
            }
        }
    }
}

pub struct Player {
    backend: Arc<Backend>,
    color: Color,
    view: ViewTransform,
    exposure: f32,
    media: Option<Media>,
    gen: u64,
    queue: Arc<Queue>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
    pub idx: usize,
    /// 1 forward, -1 reverse, 0 stopped.
    playing: i8,
    pub looping: bool,
    acc: f64,
    zoom: f32,
    pan: Vec2,
    fit: bool,
    drag: Option<(f32, usize)>,
    info: bool,
    /// The pointer is over the player, or it was clicked last: its keys win.
    pub has_keys: bool,
    focused: bool,
}

impl Player {
    pub fn new(backend: Arc<Backend>) -> Self {
        let (tx, rx) = channel();
        let queue = Arc::new(Queue::default());
        let ctx: Arc<Mutex<Option<egui::Context>>> = Arc::default();
        for _ in 0..WORKERS {
            let (q, tx, b, ctx) = (queue.clone(), tx.clone(), backend.clone(), ctx.clone());
            std::thread::spawn(move || worker(&q, &tx, &b, &ctx));
        }
        Self {
            backend,
            color: Color::default(),
            view: ViewTransform::Aces,
            exposure: 0.0,
            media: None,
            gen: 0,
            queue,
            tx,
            rx,
            ctx,
            idx: 0,
            playing: 0,
            looping: true,
            acc: 0.0,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: true,
            drag: None,
            info: false,
            has_keys: false,
            focused: false,
        }
    }

    pub fn set_color(&mut self, c: Color) {
        self.color = c;
        self.view = c.view;
        self.rebuffer();
    }

    /// Play `r` from its start.
    pub fn open(&mut self, r: &Row, color: Color) {
        self.close();
        self.color = color;
        self.view = color.view;
        self.gen += 1;
        let movie = r.kind == Kind::File && r.is_movie();
        let total = if movie { 0 } else { r.frames().max(1) };
        let (first, missing) = r.seq.as_ref().map(|s| (s.first, s.missing_frames.clone())).unwrap_or((0, vec![]));
        let m = Media {
            id: r.id,
            name: r.name.clone(),
            what: if movie { What::Movie } else { What::Sequence },
            first,
            missing,
            total,
            bytes: vec![None; total],
            failed: HashSet::new(),
            fetching: HashSet::new(),
            decoding: HashSet::new(),
            tex: HashMap::new(),
            fps_i: 1,
            speed_i: 2,
            movie_fps: 24.0,
            movie_done: !movie,
            movie_err: None,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        if movie {
            let path = self.backend.frame_paths(r.id).first().cloned().unwrap_or(r.path.clone());
            let (tx, gen, cancel, ff, ctx) = (self.tx.clone(), self.gen, m.cancel.clone(), self.backend.ffmpeg(), self.ctx.clone());
            std::thread::spawn(move || movie_stream(ff, &path, gen, &tx, &cancel, &ctx));
        }
        self.media = Some(m);
        self.idx = 0;
        self.playing = 1;
        self.acc = 0.0;
        self.fit = true;
        self.info = false;
    }

    pub fn close(&mut self) {
        if let Some(m) = self.media.take() {
            m.cancel.store(true, Ordering::SeqCst);
        }
        self.queue.tasks.lock().unwrap().clear();
        self.playing = 0;
        self.gen += 1;
    }

    /// New colour or exposure: fetch every sequence frame again.
    fn rebuffer(&mut self) {
        self.gen += 1;
        self.queue.tasks.lock().unwrap().clear();
        if let Some(m) = &mut self.media {
            if m.what == What::Sequence {
                m.bytes = vec![None; m.total];
                m.failed.clear();
                m.fetching.clear();
                m.decoding.clear();
                // Textures stay up until their replacements land.
            }
        }
    }

    /// Take finished work; schedule more. True: draw again.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(msg) = self.rx.try_recv() {
            any = true;
            self.take(msg);
        }
        self.schedule();
        any || self.playing != 0
    }

    fn take(&mut self, msg: Msg) {
        let gen = self.gen;
        let ctx = self.ctx.lock().unwrap().clone();
        let Some(m) = &mut self.media else { return };
        match msg {
            Msg::Bytes { gen: g, i, out } if g == gen => {
                m.fetching.remove(&i);
                match out {
                    Ok(b) if i < m.bytes.len() => m.bytes[i] = Some(b),
                    Ok(_) => {}
                    Err(_) => {
                        m.failed.insert(i);
                    }
                }
            }
            Msg::Image { gen: g, i, img } if g == gen || m.what == What::Movie => {
                m.decoding.remove(&i);
                if let Some(ctx) = ctx {
                    let size = img.size;
                    let t = ctx.load_texture(format!("seq.player.{i}"), img, TextureOptions::LINEAR);
                    m.tex.insert(i, (t, size));
                }
            }
            Msg::MovieMeta { gen: g, fps, total } if g == gen => {
                m.movie_fps = fps.max(1.0);
                m.total = m.total.max(total);
            }
            Msg::MovieFrame { gen: g, bytes } if g == gen => {
                m.bytes.push(Some(bytes));
                m.total = m.total.max(m.bytes.len());
            }
            Msg::MovieEnd { gen: g, err } if g == gen => {
                m.movie_done = true;
                m.movie_err = err;
                m.total = m.bytes.len().max(1);
            }
            _ => {}
        }
    }

    /// Indices from the playhead outward in the play direction, wrapping.
    fn order(&self, n: usize) -> impl Iterator<Item = usize> + '_ {
        let dir = if self.playing < 0 { -1i64 } else { 1 };
        let idx = self.idx as i64;
        (0..n as i64).map(move |k| (idx + dir * k).rem_euclid(n as i64) as usize)
    }

    fn in_window(&self, i: usize, n: usize) -> bool {
        let d = |a: usize, b: usize| (b + n - a) % n; // steps from a forward to b
        let (ahead, behind) = if self.playing < 0 { (d(i, self.idx), d(self.idx, i)) } else { (d(self.idx, i), d(i, self.idx)) };
        ahead <= AHEAD || behind <= BEHIND
    }

    fn schedule(&mut self) {
        let gen = self.gen;
        let Some(m) = &self.media else { return };
        let n = m.total.max(1);
        let mut decode: Vec<(usize, Arc<Vec<u8>>)> = vec![];
        let mut fetch: Vec<usize> = vec![];
        let busy = m.fetching.len();
        for i in self.order(n).take(AHEAD + 1) {
            if let Some(Some(b)) = m.bytes.get(i) {
                if !m.tex.contains_key(&i) && !m.decoding.contains(&i) {
                    decode.push((i, b.clone()));
                }
            }
        }
        if m.what == What::Sequence && busy < IN_FLIGHT {
            for i in self.order(n) {
                if fetch.len() + busy >= IN_FLIGHT {
                    break;
                }
                if m.bytes[i].is_none() && !m.fetching.contains(&i) && !m.failed.contains(&i) {
                    fetch.push(i);
                }
            }
        }
        // Drop textures far from the playhead.
        let far: Vec<usize> = m.tex.keys().copied().filter(|&i| !self.in_window(i, n) && i != self.idx).collect();
        let (id, color, exposure) = (m.id, Color { input: self.color.input, view: self.view }, self.exposure);
        let windows: Vec<bool> = fetch.iter().map(|&i| self.in_window(i, n)).collect();
        let m = self.media.as_mut().unwrap();
        for i in far {
            m.tex.remove(&i);
        }
        if decode.is_empty() && fetch.is_empty() {
            return;
        }
        let mut q = self.queue.tasks.lock().unwrap();
        for (i, bytes) in decode {
            m.decoding.insert(i);
            q.push_front(Task::Decode { gen, i, bytes });
        }
        for (i, w) in fetch.into_iter().zip(windows) {
            m.fetching.insert(i);
            q.push_back(Task::Fetch { gen, id, i, color, exposure, decode: w });
        }
        drop(q);
        self.queue.ready.notify_all();
    }

    fn step(&mut self, by: i64) {
        let Some(m) = &self.media else { return };
        let n = m.total.max(1) as i64;
        let next = self.idx as i64 + by;
        self.idx = if self.looping { next.rem_euclid(n) } else { next.clamp(0, n - 1) } as usize;
    }

    fn tick(&mut self, dt: f32) {
        let Some(m) = &self.media else { return };
        if self.playing == 0 {
            return;
        }
        let n = m.total.max(1);
        let fps = m.fps() as f64;
        self.acc = (self.acc + dt as f64 * fps).min(4.0);
        while self.acc >= 1.0 {
            let next = self.idx as i64 + self.playing as i64;
            let end = next < 0 || next >= n as i64;
            if end && !self.looping {
                if m.what == What::Movie && !m.movie_done {
                    break; // the stream is still coming
                }
                self.playing = 0;
                break;
            }
            let next = next.rem_euclid(n as i64) as usize;
            // Hold on a frame that isn't here yet rather than skip it.
            let m = self.media.as_ref().unwrap();
            if !m.tex.contains_key(&next) {
                break;
            }
            self.idx = next;
            self.acc -= 1.0;
        }
    }

    /// The texture to show: the playhead's, else its nearest neighbour's.
    fn shown(&self) -> Option<(&TextureHandle, [usize; 2])> {
        let m = self.media.as_ref()?;
        if let Some((t, s)) = m.tex.get(&self.idx) {
            return Some((t, *s));
        }
        m.tex.iter().min_by_key(|(i, _)| i.abs_diff(self.idx)).map(|(_, (t, s))| (t, *s))
    }
}

fn worker(q: &Queue, tx: &Sender<Msg>, b: &Backend, ctx: &Mutex<Option<egui::Context>>) {
    loop {
        let task = {
            let mut t = q.tasks.lock().unwrap();
            loop {
                if let Some(x) = t.pop_front() {
                    break x;
                }
                t = q.ready.wait(t).unwrap();
            }
        };
        match task {
            Task::Fetch { gen, id, i, color, exposure, decode } => {
                let out = b.playback_frame(id, i, MAX_DIM, color, exposure).map(Arc::new);
                if decode {
                    if let Ok(bytes) = &out {
                        if let Some(img) = jpeg(bytes) {
                            let _ = tx.send(Msg::Image { gen, i, img });
                        }
                    }
                }
                let _ = tx.send(Msg::Bytes { gen, i, out });
            }
            Task::Decode { gen, i, bytes } => {
                if let Some(img) = jpeg(&bytes) {
                    let _ = tx.send(Msg::Image { gen, i, img });
                }
            }
        }
        if let Some(c) = ctx.lock().unwrap().as_ref() {
            c.request_repaint();
        }
    }
}

fn jpeg(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// `Duration: 00:01:02.50` and `, 23.98 fps` out of ffmpeg's banner.
pub fn parse_banner(banner: &str) -> (Option<f64>, Option<f32>) {
    let dur = banner.split("Duration: ").nth(1).and_then(|s| s.get(..11)).and_then(|stamp| {
        let mut p = stamp.split(':');
        let h: f64 = p.next()?.trim().parse().ok()?;
        let m: f64 = p.next()?.parse().ok()?;
        let s: f64 = p.next()?.parse().ok()?;
        Some(h * 3600.0 + m * 60.0 + s)
    });
    let fps = banner.lines().filter(|l| l.contains("Video:")).find_map(|l| {
        let i = l.find(" fps")?;
        l[..i].rsplit([',', ' ']).next()?.trim().parse().ok()
    });
    (dur, fps)
}

/// Split concatenated JPEGs (SOI ... EOI) off the front of `buf`.
pub fn take_jpegs(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = vec![];
    loop {
        let Some(start) = buf.windows(2).position(|w| w == [0xFF, 0xD8]) else { break };
        let Some(end) = buf[start + 2..].windows(2).position(|w| w == [0xFF, 0xD9]) else { break };
        let end = start + 2 + end + 2;
        out.push(buf[start..end].to_vec());
        buf.drain(..end);
    }
    out
}

/// One ffmpeg run that streams a movie's frames as JPEGs.
fn movie_stream(ff: Option<PathBuf>, path: &Path, gen: u64, tx: &Sender<Msg>, cancel: &AtomicBool, ctx: &Mutex<Option<egui::Context>>) {
    let repaint = || {
        if let Some(c) = ctx.lock().unwrap().as_ref() {
            c.request_repaint();
        }
    };
    let Some(ff) = ff else {
        let _ = tx.send(Msg::MovieEnd { gen, err: Some("ffmpeg is unavailable".into()) });
        return;
    };
    if let Ok(o) = std::process::Command::new(&ff).arg("-hide_banner").arg("-i").arg(path).output() {
        let (dur, fps) = parse_banner(&String::from_utf8_lossy(&o.stderr));
        let fps = fps.unwrap_or(24.0);
        let total = dur.map(|d| (d * fps as f64).round() as usize).unwrap_or(0);
        let _ = tx.send(Msg::MovieMeta { gen, fps, total });
    }
    let scale = format!("scale='min({MOVIE_DIM},iw)':-2");
    let child = std::process::Command::new(&ff)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vf", &scale, "-f", "image2pipe", "-c:v", "mjpeg", "-q:v", "3", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Msg::MovieEnd { gen, err: Some(e.to_string()) });
            return;
        }
    };
    let mut out = child.stdout.take().unwrap();
    let mut buf = Vec::with_capacity(1 << 20);
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            break;
        }
        match out.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                let frames = take_jpegs(&mut buf);
                if !frames.is_empty() {
                    for f in frames {
                        let _ = tx.send(Msg::MovieFrame { gen, bytes: Arc::new(f) });
                    }
                    repaint();
                }
            }
        }
    }
    let status = child.wait();
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let failed = status.map(|s| !s.success()).unwrap_or(true) && !cancel.load(Ordering::SeqCst);
    let _ = tx.send(Msg::MovieEnd { gen, err: failed.then(|| err.lines().last().unwrap_or("ffmpeg failed").to_string()) });
    repaint();
}

// ------------------------------------------------------------------ view

pub fn show(ui: &mut Ui, app: &mut Seq) {
    let p = &mut app.player;
    if p.ctx.lock().unwrap().is_none() {
        *p.ctx.lock().unwrap() = Some(ui.ctx().clone());
    }
    let full = ui.max_rect();
    // Its keys win while it was clicked last (or the pointer is on it).
    let (pressed, hover) = ui.input(|i| (i.pointer.any_pressed(), i.pointer.hover_pos()));
    if pressed {
        p.focused = hover.is_some_and(|h| full.contains(h));
    }
    p.has_keys = p.media.is_some() && (p.focused || hover.is_some_and(|h| full.contains(h))) && !ui.ctx().egui_wants_keyboard_input();
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    p.tick(dt);
    if p.playing != 0 {
        ui.ctx().request_repaint();
    }
    header(ui, app);
    let p = &mut app.player;
    let Some(m) = &p.media else {
        ui.centered_and_justified(|ui| ui.weak("Nothing playing. Double-click a sequence or movie, or press Space on it."));
        return;
    };
    let transport_h = 58.0;
    let avail = ui.available_rect_before_wrap();
    let view = Rect::from_min_max(avail.min, pos2(avail.max.x, avail.max.y - transport_h));
    let (_, resp) = ui.allocate_exact_size(view.size(), Sense::click_and_drag());
    let painter = ui.painter_at(view);
    painter.rect_filled(view, 0.0, Color32::from_gray(12));
    let total = m.total.max(1);
    let shown = p.shown().map(|(t, s)| (t.id(), s));
    if let Some((tex, size)) = shown {
        let img = vec2(size[0] as f32, size[1] as f32);
        if p.fit {
            p.zoom = imfram::imageview::fit_zoom(view.size(), img).clamp(0.02, 64.0);
            p.pan = Vec2::ZERO;
            p.fit = false;
        }
        let r = imfram::imageview::image_rect(view, img, p.zoom, p.pan, ui.pixels_per_point());
        painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else {
        let m = p.media.as_ref().unwrap();
        let t = match &m.movie_err {
            Some(e) => e.clone(),
            None => "buffering…".into(),
        };
        painter.text(view.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(14.0), Color32::from_gray(160));
    }
    // Hands: left-drag scrubs, middle/right-drag pans, alt-drag zooms, the wheel zooms.
    let alt = ui.input(|i| i.modifiers.alt);
    if resp.drag_started_by(PointerButton::Primary) && !alt {
        p.drag = Some((resp.interact_pointer_pos().map(|q| q.x).unwrap_or(0.0), p.idx));
        p.playing = 0;
    }
    if resp.dragged_by(PointerButton::Primary) {
        if alt {
            let at = ui.input(|i| i.pointer.press_origin()).unwrap_or(view.center());
            let f = (resp.drag_delta().x * 0.005).exp();
            (p.pan, p.zoom) = imfram::imageview::zoom_about(p.pan, p.zoom, at - view.center(), f);
            p.zoom = p.zoom.clamp(0.02, 64.0);
        } else if let (Some((x0, i0)), Some(q)) = (p.drag, resp.interact_pointer_pos()) {
            let per = (view.width() / total as f32).clamp(3.0, 20.0);
            let d = ((q.x - x0) / per).round() as i64;
            p.idx = if p.looping { (i0 as i64 + d).rem_euclid(total as i64) } else { (i0 as i64 + d).clamp(0, total as i64 - 1) } as usize;
        }
    }
    if resp.drag_stopped() {
        p.drag = None;
    }
    if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) {
        p.pan += resp.drag_delta();
    }
    if resp.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            let at = resp.hover_pos().unwrap_or(view.center());
            let f = (dy * 0.0015).exp();
            (p.pan, p.zoom) = imfram::imageview::zoom_about(p.pan, p.zoom, at - view.center(), f);
            p.zoom = p.zoom.clamp(0.02, 64.0);
        }
    }
    if p.info {
        info_overlay(ui, app, view);
    }
    transport(ui, app);
    keys(ui, app);
}

fn header(ui: &mut Ui, app: &mut Seq) {
    let p = &mut app.player;
    ui.horizontal(|ui| {
        imfram::dock::grip(ui);
        let Some(m) = &p.media else { return };
        ui.strong(&m.name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("×").on_hover_text("close (Esc)").clicked() {
                p.close();
                return;
            }
            if ui.selectable_label(p.info, "Info").on_hover_text("I").clicked() {
                p.info = !p.info;
            }
            if ui.button("1:1").on_hover_text("1").clicked() {
                (p.pan, p.zoom) = imfram::imageview::zoom_about(p.pan, p.zoom, Vec2::ZERO, 1.0 / p.zoom);
            }
            if ui.button("fit").on_hover_text("F").clicked() {
                p.fit = true;
            }
            ui.monospace(format!("{:>5.0}%", p.zoom * 100.0));
        });
    });
}

fn transport(ui: &mut Ui, app: &mut Seq) {
    let p = &mut app.player;
    let Some(m) = &p.media else { return };
    let total = m.total.max(1);
    // The track: buffered frames behind, the playhead on top; drag to scrub.
    let (track, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
    let painter = ui.painter_at(track);
    let vis = ui.visuals();
    painter.rect_filled(track, 3.0, vis.extreme_bg_color);
    let green = Color32::from_rgb(70, 140, 80);
    let cols = (track.width() as usize).max(1);
    let mut x0 = None;
    for c in 0..=cols {
        let have = c < cols && {
            let a = c * total / cols;
            let b = ((c + 1) * total / cols).max(a + 1).min(total);
            (a..b).all(|i| m.bytes.get(i).is_some_and(|x| x.is_some()))
        };
        match (have, x0) {
            (true, None) => x0 = Some(c),
            (false, Some(s)) => {
                let r = Rect::from_min_max(pos2(track.left() + s as f32, track.center().y - 2.0), pos2(track.left() + c as f32, track.center().y + 2.0));
                painter.rect_filled(r, 0.0, green);
                x0 = None;
            }
            _ => {}
        }
    }
    let x = track.left() + (p.idx as f32 + 0.5) / total as f32 * track.width();
    painter.rect_filled(Rect::from_center_size(pos2(x, track.center().y), vec2(3.0, track.height())), 1.0, vis.selection.bg_fill);
    if let Some(q) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        let f = ((q.x - track.left()) / track.width()).clamp(0.0, 0.9999);
        p.idx = (f * total as f32) as usize;
        if resp.drag_started() {
            p.playing = 0;
        }
    }
    let m = p.media.as_ref().unwrap();
    let what = m.what;
    let (loaded, failed, name) = (m.loaded(), m.failed.len(), m.frame_name(p.idx));
    let movie_streaming = what == What::Movie && !m.movie_done;
    ui.horizontal(|ui| {
        if ui.button("⏮").on_hover_text("first (Home)").clicked() {
            p.idx = 0;
        }
        if ui.selectable_label(p.playing < 0, "◀").on_hover_text("play backwards (J)").clicked() {
            p.playing = if p.playing < 0 { 0 } else { -1 };
        }
        if ui.button(if p.playing != 0 { "⏸" } else { "▶" }).on_hover_text("play / pause (Space; L forward, K stop)").clicked() {
            p.playing = if p.playing != 0 { 0 } else { 1 };
        }
        if ui.button("‹").on_hover_text("back a frame (←)").clicked() {
            p.playing = 0;
            p.step(-1);
        }
        if ui.button("›").on_hover_text("forward a frame (→)").clicked() {
            p.playing = 0;
            p.step(1);
        }
        if ui.button("⏭").on_hover_text("last (End)").clicked() {
            p.idx = total - 1;
        }
        ui.monospace(format!("{name}  {}/{total}", p.idx + 1));
        ui.separator();
        let m = p.media.as_mut().unwrap();
        match what {
            What::Sequence => {
                egui::ComboBox::from_id_salt("seq.fps").width(64.0).selected_text(format!("{} fps", FPS[m.fps_i])).show_ui(ui, |ui| {
                    for (i, f) in FPS.iter().enumerate() {
                        ui.selectable_value(&mut m.fps_i, i, format!("{f} fps"));
                    }
                });
            }
            What::Movie => {
                egui::ComboBox::from_id_salt("seq.speed").width(64.0).selected_text(format!("{}×", SPEEDS[m.speed_i])).show_ui(ui, |ui| {
                    for (i, s) in SPEEDS.iter().enumerate() {
                        ui.selectable_value(&mut m.speed_i, i, format!("{s}×"));
                    }
                });
                ui.weak(format!("{:.3} fps", m.movie_fps));
            }
        }
        ui.checkbox(&mut p.looping, "loop");
        if what == What::Sequence {
            ui.separator();
            let mut v = p.view;
            egui::ComboBox::from_id_salt("seq.view").width(80.0).selected_text(view_name(v)).show_ui(ui, |ui| {
                for t in [ViewTransform::Aces, ViewTransform::Gamma22, ViewTransform::Raw] {
                    ui.selectable_value(&mut v, t, view_name(t));
                }
            });
            let mut e = p.exposure;
            ui.add(egui::Slider::new(&mut e, -4.0..=4.0).step_by(0.5).text("exp"));
            let settled = !ui.input(|i| i.pointer.any_down());
            if v != p.view || (e != p.exposure && settled) || (e != p.exposure && (e - p.exposure).abs() >= 0.5) {
                p.view = v;
                p.exposure = e;
                p.rebuffer();
            }
        }
        ui.separator();
        let mut t = format!("{loaded}/{total} loaded");
        if movie_streaming {
            t.push_str(" (decoding)");
        }
        if failed > 0 {
            t.push_str(&format!(" · {failed} failed"));
        }
        ui.weak(t);
    });
}

fn view_name(v: ViewTransform) -> &'static str {
    match v {
        ViewTransform::Aces => "ACES",
        ViewTransform::Gamma22 => "sRGB",
        ViewTransform::Raw => "Raw",
    }
}

fn info_overlay(ui: &mut Ui, app: &mut Seq, view: Rect) {
    let Some(id) = app.player.media.as_ref().map(|m| m.id) else { return };
    app.want_info(id);
    let r = Rect::from_min_size(view.right_top() + vec2(-300.0, 8.0), vec2(292.0, view.height() - 16.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    egui::Frame::popup(child.style()).show(&mut child, |ui| {
        ui.set_width(270.0);
        match app.info.get(&id) {
            Some(Ok(i)) => {
                ui.strong(&i.format);
                egui::Grid::new("seq.pinfo").num_columns(2).show(ui, |ui| {
                    for (k, v) in &i.fields {
                        ui.weak(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            }
            Some(Err(e)) => {
                ui.weak(e);
            }
            None => {
                ui.weak("reading…");
            }
        }
    });
}

fn keys(ui: &mut Ui, app: &mut Seq) {
    let p = &mut app.player;
    if !p.has_keys {
        return;
    }
    let k = |key: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
    if k(Key::Escape) {
        if p.info {
            p.info = false;
        } else {
            p.close();
            return;
        }
    }
    if k(Key::Space) {
        p.playing = if p.playing != 0 { 0 } else { 1 };
    }
    if k(Key::L) {
        p.playing = 1;
    }
    if k(Key::J) {
        p.playing = -1;
    }
    if k(Key::K) {
        p.playing = 0;
    }
    if k(Key::ArrowLeft) {
        p.playing = 0;
        p.step(-1);
    }
    if k(Key::ArrowRight) {
        p.playing = 0;
        p.step(1);
    }
    if k(Key::Home) {
        p.idx = 0;
    }
    if k(Key::End) {
        p.idx = p.media.as_ref().map(|m| m.total.max(1) - 1).unwrap_or(0);
    }
    if k(Key::I) {
        p.info = !p.info;
    }
    if k(Key::F) {
        p.fit = true;
    }
    if k(Key::Num1) {
        (p.pan, p.zoom) = imfram::imageview::zoom_about(p.pan, p.zoom, Vec2::ZERO, 1.0 / p.zoom);
    }
    if k(Key::Equals) || k(Key::Plus) {
        p.zoom = (p.zoom * 1.25).min(64.0);
    }
    if k(Key::Minus) {
        p.zoom = (p.zoom / 1.25).max(0.02);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_gives_duration_and_fps() {
        let b = "  Duration: 00:00:02.00, start: 0.000000, bitrate: 1 kb/s\n  Stream #0:0: Video: h264 (High), yuv420p, 640x360, 23.98 fps, 24 tbr\n";
        let (d, f) = parse_banner(b);
        assert_eq!(d, Some(2.0));
        assert_eq!(f, Some(23.98));
    }

    #[test]
    fn concatenated_jpegs_split_and_a_partial_one_waits() {
        let mut buf = vec![0xFF, 0xD8, 1, 2, 0xFF, 0xD9, 0xFF, 0xD8, 3, 0xFF, 0xD9, 0xFF, 0xD8, 4];
        let got = take_jpegs(&mut buf);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1], vec![0xFF, 0xD8, 3, 0xFF, 0xD9]);
        assert_eq!(buf, vec![0xFF, 0xD8, 4]);
    }
}
