//! `eden serve`: a low-latency interactive browser player.
//!
//! The runtime stays on the main thread (it owns the software renderer and
//! the VM). HTTP worker threads only exchange commands and PNG frames with
//! it through channels, so no locking of engine state is needed.
//!
//! * `GET /stream` is a `multipart/x-mixed-replace` PNG stream consumed by a
//!   plain `<img>` element.
//! * `POST /input` maps pointer/wheel/keyboard events onto the same
//!   [`eden_render::ui::UiModel`] hit regions used by the iOS host.
//! * `GET /audio/clip?name=...` serves the original OGG/WAV entry straight
//!   out of `bgm.paz` / `se.paz` / `voice.paz` / `sys.paz`.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eden_ffi::Runtime;
use eden_paz::{Edition, PazArchive};
use eden_render::ui::{UiCommand, UiScreen};

use crate::{game_dir, pm_dir, Opts};

const HTML: &str = include_str!("serve.html");

enum Command {
    Move(i32, i32),
    Tap(i32, i32),
    Right(i32, i32),
    Wheel(i32),
    Action(String, Option<usize>),
    Snapshot(mpsc::Sender<Vec<u8>>),
}

#[derive(Clone)]
struct EventView {
    seq: u64,
    kind: String,
    name: String,
}

#[derive(Default)]
struct AudioLog {
    seq: u64,
    events: VecDeque<EventView>,
}

#[derive(Default)]
struct Status {
    screen: String,
    edition: String,
    title_edition: usize,
    bgm: Option<String>,
    bgm_progress: f32,
    auto: bool,
    skip: bool,
    waiting: bool,
    vm: String,
    frames: u64,
    fps: f32,
    encode_ms: f32,
    error: Option<String>,
}

struct Shared {
    cmd_tx: mpsc::Sender<Command>,
    subscribers: Mutex<Vec<SyncSender<Arc<Vec<u8>>>>>,
    latest: Mutex<Option<Arc<Vec<u8>>>>,
    status: Mutex<Status>,
    audio: Mutex<AudioLog>,
    cache: Mutex<HashMap<String, Option<Arc<Vec<u8>>>>>,
    main_dir: PathBuf,
    pm_dir: PathBuf,
    running: AtomicBool,
}

impl Shared {
    fn broadcast(&self, frame: Arc<Vec<u8>>) {
        *self.latest.lock().unwrap() = Some(frame.clone());
        let mut subs = self.subscribers.lock().unwrap();
        // Bounded queues drop stale frames instead of building latency.
        subs.retain(|tx| match tx.try_send(frame.clone()) {
            Ok(()) | Err(TrySendError::Full(_)) => true,
            Err(TrySendError::Disconnected(_)) => false,
        });
    }

    fn subscribe(&self) -> mpsc::Receiver<Arc<Vec<u8>>> {
        let (tx, rx) = mpsc::sync_channel(1);
        if let Some(frame) = self.latest.lock().unwrap().clone() {
            let _ = tx.try_send(frame);
        }
        self.subscribers.lock().unwrap().push(tx);
        rx
    }

    fn push_audio(&self, kind: &str, name: &str) {
        let mut log = self.audio.lock().unwrap();
        log.seq += 1;
        let seq = log.seq;
        log.events.push_back(EventView {
            seq,
            kind: kind.to_string(),
            name: name.to_string(),
        });
        while log.events.len() > 96 {
            log.events.pop_front();
        }
    }

    /// Resolve an audio entry name against the retail archives, preferring the
    /// edition that is currently loaded.
    fn audio_entry(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            return None;
        }
        let edition = self.status.lock().unwrap().edition.clone();
        let key = format!("{edition}/{name}");
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let mut dirs = vec![self.main_dir.clone()];
        if self.pm_dir.exists() {
            if edition == "pm" {
                dirs.swap(0, 1);
            }
            dirs.insert(1, self.pm_dir.clone());
        }
        let mut found = None;
        'outer: for dir in dirs {
            for archive in ["bgm.paz", "se.paz", "voice.paz", "sys.paz"] {
                let path = dir.join(archive);
                if !path.exists() {
                    continue;
                }
                let Ok(archive) = PazArchive::open(&path) else {
                    continue;
                };
                if archive.entry(name).is_none() {
                    continue;
                }
                if let Ok(bytes) = archive.read_entry(name) {
                    found = Some(Arc::new(bytes));
                    break 'outer;
                }
            }
        }
        self.cache
            .lock()
            .unwrap()
            .insert(key, found.clone());
        found
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    if !game.exists() {
        return Err(format!("game dir {} does not exist", game.display()));
    }
    let bind = opts.value("bind").unwrap_or("127.0.0.1:8787").to_string();
    let save_dir = PathBuf::from(opts.value("save-dir").unwrap_or("reports/saves"));
    let fps = opts.number("fps", 30)?.clamp(5, 60) as u64;
    let start_pm = opts.flag("pm");

    std::fs::create_dir_all(&save_dir)
        .map_err(|e| format!("{}: {e}", save_dir.display()))?;

    let mut runtime = Runtime::open(&game, None).map_err(|e| e.to_string())?;
    runtime
        .set_save_dir(&save_dir)
        .map_err(|e| e.to_string())?;
    runtime.use_system_font();
    runtime.set_auto_advance(false);
    runtime.refresh_ui_lists();
    if start_pm {
        runtime
            .open_edition(Edition::PlusMosaic)
            .map_err(|e| e.to_string())?;
    }
    runtime.show_title();

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        cmd_tx,
        subscribers: Mutex::new(Vec::new()),
        latest: Mutex::new(None),
        status: Mutex::new(Status {
            edition: edition_name(runtime.edition()).to_string(),
            ..Status::default()
        }),
        audio: Mutex::new(AudioLog::default()),
        cache: Mutex::new(HashMap::new()),
        main_dir: game.clone(),
        pm_dir: pm_dir(&game),
        running: AtomicBool::new(true),
    });

    let listener = TcpListener::bind(&bind).map_err(|e| format!("bind {bind}: {e}"))?;
    let local = listener
        .local_addr()
        .map_err(|e| e.to_string())?;
    println!("eden serve: http://{local}/  (Ctrl-C to stop)");
    println!("  game: {}", game.display());
    println!("  saves: {}", save_dir.display());
    if opts.flag("open") {
        let url = format!("http://{local}/");
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }

    {
        let shared = shared.clone();
        thread::spawn(move || accept_loop(listener, shared));
    }

    game_loop(&mut runtime, &shared, &cmd_rx, fps);
    Ok(())
}

fn edition_name(edition: Edition) -> &'static str {
    match edition {
        Edition::Main => "main",
        Edition::PlusMosaic => "pm",
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let shared = shared.clone();
        thread::spawn(move || {
            let _ = handle_connection(stream, &shared);
        });
    }
}

fn game_loop(
    runtime: &mut Runtime,
    shared: &Arc<Shared>,
    cmd_rx: &mpsc::Receiver<Command>,
    fps: u64,
) {
    let frame_interval = Duration::from_micros(1_000_000 / fps.max(1));
    let mut last_tick = Instant::now();
    let mut last_encode = Instant::now() - frame_interval;
    let mut fps_mark = Instant::now();
    let mut fps_frames = 0u32;
    let mut frames = 0u64;
    let mut encode_ms = 0.0f32;

    while shared.running.load(Ordering::Relaxed) {
        while let Ok(command) = cmd_rx.try_recv() {
            apply_command(runtime, command, shared);
        }

        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f64().min(0.1);
        last_tick = now;
        if let Err(e) = runtime.tick_display(dt) {
            shared.status.lock().unwrap().error = Some(e.to_string());
        }

        for event in runtime.drain_audio_events() {
            shared.push_audio(event.kind, &event.name);
        }

        if now.duration_since(last_encode) >= frame_interval {
            let started = Instant::now();
            runtime.render_now();
            // Raw RGBA is far cheaper than PNG on a loopback link: 2.6 MB per
            // frame at 30 fps is ~78 MB/s, and the browser draws it with a
            // single `putImageData`.
            shared.broadcast(Arc::new(runtime.frame().data.clone()));
            frames += 1;
            fps_frames += 1;
            encode_ms = started.elapsed().as_secs_f32() * 1000.0;
            last_encode = Instant::now();
        }

        if now.duration_since(fps_mark) >= Duration::from_secs(1) {
            let elapsed = now.duration_since(fps_mark).as_secs_f32();
            let mut status = shared.status.lock().unwrap();
            status.fps = fps_frames as f32 / elapsed;
            status.frames = frames;
            status.encode_ms = encode_ms;
            status.screen = format!("{:?}", runtime.ui().screen);
            status.edition = edition_name(runtime.edition()).to_string();
            status.title_edition = runtime.ui().title_edition;
            status.bgm = runtime.bgm_name().map(str::to_string);
            let (pos, total) = runtime.bgm_position();
            status.bgm_progress = if total > 0 {
                pos as f32 / total as f32
            } else {
                0.0
            };
            status.auto = runtime.ui().auto;
            status.skip = runtime.ui().skip;
            status.waiting = runtime.ui().waiting;
            status.vm = runtime
                .vm_state()
                .map(|state| format!("{state:?}"))
                .unwrap_or_else(|| "None".to_string());
            drop(status);
            fps_mark = now;
            fps_frames = 0;
        }

        thread::sleep(Duration::from_millis(2));
    }
}

fn apply_command(runtime: &mut Runtime, command: Command, shared: &Shared) {
    match command {
        Command::Move(x, y) => {
            runtime.ui_mut().move_cursor(x, y);
            // The browser draws the pointer; the software cursor is only for
            // the headless screenshots.
            runtime.ui_mut().cursor_visible = false;
        }
        Command::Tap(x, y) => {
            runtime.ui_tap(x, y);
            runtime.ui_mut().cursor_visible = false;
        }
        Command::Right(x, y) => {
            runtime.ui_right_click(x, y);
            runtime.ui_mut().cursor_visible = false;
        }
        Command::Wheel(delta) => {
            if runtime.ui().screen == UiScreen::Game && delta < 0 {
                runtime.ui_swipe_up();
            } else if runtime.ui().screen == UiScreen::Backlog {
                runtime.ui_scroll(if delta > 0 { 3 } else { -3 });
            }
        }
        Command::Snapshot(reply) => {
            let _ = reply.send(eden_paz::encode_png(runtime.frame()).unwrap_or_default());
        }
        Command::Action(action, slot) => match action.as_str() {
            "new" => {
                let edition = if runtime.ui().title_edition == 1 {
                    Edition::PlusMosaic
                } else {
                    Edition::Main
                };
                if runtime.edition() != edition
                    && runtime.open_edition(edition).is_ok()
                {
                    runtime.refresh_ui_lists();
                }
                if let Err(e) = runtime.start_script("A001.sc") {
                    shared.status.lock().unwrap().error = Some(e.to_string());
                }
            }
            "continue" => {
                if let Some(index) = runtime.latest_slot() {
                    if let Err(e) = runtime.load_slot(index) {
                        shared.status.lock().unwrap().error = Some(e.to_string());
                    }
                } else {
                    runtime.ui_mut().toast("No save data");
                }
            }
            "main" => {
                if let Err(e) = runtime.open_edition(Edition::Main) {
                    shared.status.lock().unwrap().error = Some(e.to_string());
                }
                runtime.show_title();
            }
            "pm" => {
                if let Err(e) = runtime.open_edition(Edition::PlusMosaic) {
                    shared.status.lock().unwrap().error = Some(e.to_string());
                }
                runtime.show_title();
            }
            "title" => runtime.show_title(),
            "save" => runtime.open_ui(UiScreen::Save),
            "load" => runtime.open_ui(UiScreen::Load),
            "backlog" => runtime.open_ui(UiScreen::Backlog),
            "config" => runtime.open_ui(UiScreen::Config),
            "extra" => runtime.open_ui(UiScreen::Extra),
            "music" => runtime.open_ui(UiScreen::Music),
            "back" => runtime.handle_ui_command(UiCommand::Back),
            "auto" => {
                let next = !runtime.ui().auto;
                runtime.handle_ui_command(UiCommand::SetAuto(next));
            }
            "skip" => {
                let next = !runtime.ui().skip;
                runtime.handle_ui_command(UiCommand::SetSkip(next));
            }
            "save_slot" => {
                if let Err(e) = runtime.save_slot(slot.unwrap_or(0)) {
                    shared.status.lock().unwrap().error = Some(e.to_string());
                }
            }
            "load_slot" => {
                if let Err(e) = runtime.load_slot(slot.unwrap_or(0)) {
                    shared.status.lock().unwrap().error = Some(e.to_string());
                }
            }
            other => {
                shared.status.lock().unwrap().error = Some(format!("unknown action {other:?}"));
            }
        },
    }
}

fn handle_connection(stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let Some(request) = read_request(&mut reader)? else {
        return Ok(());
    };
    let mut stream = stream;
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            write_response(&mut stream, "text/html; charset=utf-8", HTML.as_bytes())
        }
        ("GET", "/rawstream") => stream_frames(&mut stream, shared),
        ("GET", "/api/state") => {
            let since: u64 = request
                .query
                .get("since")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let body = state_json(shared, since);
            write_response(&mut stream, "application/json", body.as_bytes())
        }
        ("GET", "/api/frame.png") => {
            let (tx, rx) = mpsc::channel();
            let _ = shared.cmd_tx.send(Command::Snapshot(tx));
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(png) if !png.is_empty() => write_response(&mut stream, "image/png", &png),
                _ => write_response(&mut stream, "text/plain", b"no frame yet"),
            }
        }
        ("GET", "/audio/clip") => {
            let name = request.query.get("name").cloned().unwrap_or_default();
            match shared.audio_entry(&name) {
                Some(bytes) => {
                    let mime = if name.to_ascii_lowercase().ends_with(".ogg") {
                        "audio/ogg"
                    } else if name.to_ascii_lowercase().ends_with(".wav") {
                        "audio/wav"
                    } else {
                        "application/octet-stream"
                    };
                    write_response(&mut stream, mime, &bytes)
                }
                None => write_response(&mut stream, "text/plain", b"missing audio entry"),
            }
        }
        ("POST", "/input") => {
            let value: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or_default();
            let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let x = value.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let y = value.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let delta = value.get("delta").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let command = match kind {
                "move" => Command::Move(x, y),
                "tap" => Command::Tap(x, y),
                "right" => Command::Right(x, y),
                "wheel" => Command::Wheel(delta),
                _ => Command::Move(x, y),
            };
            let _ = shared.cmd_tx.send(command);
            write_response(&mut stream, "application/json", b"{\"ok\":true}")
        }
        ("POST", "/api/action") => {
            let value: serde_json::Value =
                serde_json::from_slice(&request.body).unwrap_or_default();
            let action = value
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let slot = value
                .get("slot")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let _ = shared.cmd_tx.send(Command::Action(action, slot));
            write_response(&mut stream, "application/json", b"{\"ok\":true}")
        }
        ("GET", "/favicon.ico") => {
            stream.write_all(
                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
        }
        _ => write_response(&mut stream, "text/plain", b"not found"),
    }
}

fn stream_frames(stream: &mut TcpStream, shared: &Shared) -> std::io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\n\
          Content-Type: application/octet-stream\r\n\
          Cache-Control: no-cache, no-store, must-revalidate\r\n\
          Connection: close\r\n\r\n",
    )?;
    let rx = shared.subscribe();
    while let Ok(frame) = rx.recv_timeout(Duration::from_secs(5)) {
        stream.write_all(&(frame.len() as u32).to_le_bytes())?;
        stream.write_all(&frame)?;
        stream.flush()?;
    }
    Ok(())
}

fn state_json(shared: &Shared, since: u64) -> String {
    let status = shared.status.lock().unwrap();
    let events: Vec<serde_json::Value> = {
        let log = shared.audio.lock().unwrap();
        log.events
            .iter()
            .filter(|event| event.seq > since)
            .map(|event| {
                serde_json::json!({
                    "seq": event.seq,
                    "kind": event.kind,
                    "name": event.name,
                })
            })
            .collect()
    };
    let seq = shared.audio.lock().unwrap().seq;
    serde_json::json!({
        "screen": status.screen,
        "edition": status.edition,
        "title_edition": status.title_edition,
        "bgm": status.bgm,
        "bgm_progress": status.bgm_progress,
        "auto": status.auto,
        "skip": status.skip,
        "waiting": status.waiting,
        "vm": status.vm,
        "frames": status.frames,
        "fps": status.fps,
        "encode_ms": status.encode_ms,
        "error": status.error,
        "seq": seq,
        "events": events,
    })
    .to_string()
}

fn write_response(stream: &mut TcpStream, mime: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_query(query)),
        None => (target, HashMap::new()),
    };
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((key, value)) = header.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_length.min(1 << 20)];
    if !body.is_empty() {
        reader.read_exact(&mut body)?;
    }
    Ok(Some(Request {
        method,
        path,
        query,
        body,
    }))
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        out.insert(percent_decode(key), percent_decode(value));
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(value) => {
                        out.push(value);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
