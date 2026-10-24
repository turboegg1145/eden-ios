//! Safe Rust runtime shared by the desktop CLI and the iOS C ABI.
//!
//! The runtime owns the archives, the parsed script set, the 60 Hz VM, the
//! scene graph, the deterministic software renderer and the audio mixer. It
//! performs no network I/O and never converts game assets: every pixel and
//! every sample comes from the retail `.paz` files.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::collections::BTreeSet;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use eden_audio::{
    decode_paz_entry, AudioClip, AudioError, AudioPlayer, LipSyncAnalyzer, PlaybackParams,
};
use eden_paz::{AniFile, PazArchive, PazEntry, RgbaImage};
use eden_render::ui::{
    self, GalleryEntry, MusicEntry, SaveSlot, UiAssets, UiCommand, UiModel, UiScreen,
};
use eden_render::{MessageOverlay, Scene, ShakeKind, SoftwareRenderer, TextureCache, Transition};
use eden_script::{
    parse_script_bytes, MemoryScripts, ScrollKind, ScriptEncoding, ScriptError, ScriptVm, VmEvent,
    VmState,
};
use eden_video::avi::AviFile;
use eden_video::{open_decoder, VideoDecoder};

/// Errors surfaced by the runtime.
#[derive(Debug)]
pub enum RuntimeError {
    Paz(eden_paz::PazError),
    Script(ScriptError),
    Audio(AudioError),
    Video(String),
    Io(String),
    Missing(String),
    Parse(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::Paz(e) => write!(f, "paz: {e}"),
            RuntimeError::Script(e) => write!(f, "script: {e}"),
            RuntimeError::Audio(e) => write!(f, "audio: {e}"),
            RuntimeError::Video(e) => write!(f, "video: {e}"),
            RuntimeError::Io(e) => write!(f, "io: {e}"),
            RuntimeError::Missing(e) => write!(f, "missing: {e}"),
            RuntimeError::Parse(e) => write!(f, "parse: {e}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<eden_paz::PazError> for RuntimeError {
    fn from(value: eden_paz::PazError) -> Self {
        RuntimeError::Paz(value)
    }
}

impl From<ScriptError> for RuntimeError {
    fn from(value: ScriptError) -> Self {
        RuntimeError::Script(value)
    }
}

impl From<AudioError> for RuntimeError {
    fn from(value: AudioError) -> Self {
        RuntimeError::Audio(value)
    }
}

impl From<eden_video::avi::AviError> for RuntimeError {
    fn from(value: eden_video::avi::AviError) -> Self {
        RuntimeError::Video(value.to_string())
    }
}

impl From<std::io::Error> for RuntimeError {
    fn from(value: std::io::Error) -> Self {
        RuntimeError::Io(value.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Read-only view over the archives plus a byte-bounded decoded-image cache.
pub struct Assets {
    pub scr: PazArchive,
    pub bg: PazArchive,
    pub sys: PazArchive,
    pub st: Option<PazArchive>,
    pub bgm: Option<PazArchive>,
    pub se: Option<PazArchive>,
    pub voice: Option<PazArchive>,
    pub mov: Option<PazArchive>,
    images: TextureCache,
    anis: HashMap<String, Arc<AniFile>>,
}

fn open_archive(path: &Path, exe: Option<&Path>) -> Result<PazArchive> {
    match exe {
        Some(exe) => Ok(PazArchive::open_with_exe(path, exe)?),
        None => Ok(PazArchive::open(path)?),
    }
}

fn open_optional(path: &Path, exe: Option<&Path>) -> Result<Option<PazArchive>> {
    if path.exists() {
        Ok(Some(open_archive(path, exe)?))
    } else {
        Ok(None)
    }
}

impl Assets {
    pub fn open(dir: &Path, exe: Option<&Path>) -> Result<Self> {
        let scr_path = dir.join("scr.paz");
        let bg_path = dir.join("bg.paz");
        let sys_path = dir.join("sys.paz");
        for path in [&scr_path, &bg_path, &sys_path] {
            if !path.exists() {
                return Err(RuntimeError::Missing(path.display().to_string()));
            }
        }
        Ok(Self {
            scr: open_archive(&scr_path, exe)?,
            bg: open_archive(&bg_path, exe)?,
            sys: open_archive(&sys_path, exe)?,
            st: open_optional(&dir.join("st.paz"), exe)?,
            bgm: open_optional(&dir.join("bgm.paz"), exe)?,
            se: open_optional(&dir.join("se.paz"), exe)?,
            voice: open_optional(&dir.join("voice.paz"), exe)?,
            mov: open_optional(&dir.join("mov.paz"), exe)?,
            // 192 MiB is inside the 128–256 MiB iOS budget documented in
            // TARGET_IOS_CONSTRAINTS.md.
            images: TextureCache::new(192 * 1024 * 1024),
            anis: HashMap::new(),
        })
    }

    fn locate(&self, name: &str) -> Option<&PazArchive> {
        if self.bg.entry(name).is_some() {
            return Some(&self.bg);
        }
        if self.sys.entry(name).is_some() {
            return Some(&self.sys);
        }
        if let Some(st) = &self.st {
            if st.entry(name).is_some() {
                return Some(st);
            }
        }
        None
    }

    pub fn has(&self, name: &str) -> bool {
        self.locate(name).is_some()
    }

    pub fn image(&mut self, name: &str) -> Result<Arc<RgbaImage>> {
        if let Some(image) = self.images.get(name) {
            return Ok(image);
        }
        let image = {
            let archive = self
                .locate(name)
                .ok_or_else(|| RuntimeError::Missing(format!("image {name:?}")))?;
            Arc::new(archive.decode_png(name)?)
        };
        self.images.insert(name.to_string(), image.clone());
        Ok(image)
    }

    pub fn ani(&mut self, name: &str) -> Result<Arc<AniFile>> {
        if let Some(ani) = self.anis.get(name) {
            return Ok(ani.clone());
        }
        let ani = {
            let archive = self
                .locate(name)
                .ok_or_else(|| RuntimeError::Missing(format!("animation {name:?}")))?;
            Arc::new(archive.parse_ani(name)?)
        };
        self.anis.insert(name.to_string(), ani.clone());
        Ok(ani)
    }

    /// Returns the same-named `.ani` for an image, when the retail data has
    /// one. 1,720 of the 2,407 `bg.paz` PNGs have a differential animation.
    pub fn ani_for_image(&mut self, image_name: &str) -> Result<Option<Arc<AniFile>>> {
        let stem = image_name
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(image_name);
        let name = format!("{stem}.ani");
        if !self.has(&name) {
            return Ok(None);
        }
        Ok(Some(self.ani(&name)?))
    }

    /// Stream a `mov.paz` entry to a temporary file so external decoders
    /// (FFmpeg / AVFoundation) can seek it without keeping 100+ MiB in RAM.
    pub fn video_file(&mut self, name: &str) -> Result<PathBuf> {
        let archive = self
            .mov
            .as_ref()
            .ok_or_else(|| RuntimeError::Missing("mov.paz".into()))?;
        let entry = archive
            .entry(name)
            .ok_or_else(|| RuntimeError::Missing(format!("movie {name:?}")))?
            .clone();
        let dir = std::env::temp_dir().join("eden-video-cache");
        std::fs::create_dir_all(&dir)?;
        let ident: String = archive
            .ident()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let stem: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' })
            .collect();
        let path = dir.join(format!("{ident}-{stem}"));
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() == u64::from(entry.unpacked_size) {
                return Ok(path);
            }
        }
        let mut reader = archive.open_entry(&entry)?;
        let mut file = std::fs::File::create(&path)?;
        std::io::copy(&mut reader, &mut file)?;
        file.flush()?;
        Ok(path)
    }

    pub fn image_cache_used(&self) -> usize {
        self.images.used()
    }

    pub fn image_cache_len(&self) -> usize {
        self.images.len()
    }

    pub fn entry(&self, archive: &str, name: &str) -> Option<&PazEntry> {
        match archive {
            "scr" => self.scr.entry(name),
            "bg" => self.bg.entry(name),
            "sys" => self.sys.entry(name),
            "st" => self.st.as_ref().and_then(|a| a.entry(name)),
            "bgm" => self.bgm.as_ref().and_then(|a| a.entry(name)),
            "se" => self.se.as_ref().and_then(|a| a.entry(name)),
            "voice" => self.voice.as_ref().and_then(|a| a.entry(name)),
            "mov" => self.mov.as_ref().and_then(|a| a.entry(name)),
            _ => None,
        }
    }
}

/// Parse every `.sc` in `scr.paz` using the encoding of the archive edition.
pub fn load_scripts(archive: &PazArchive) -> Result<MemoryScripts> {
    let encoding = match archive.edition() {
        eden_paz::Edition::Main => ScriptEncoding::Gbk,
        eden_paz::Edition::PlusMosaic => ScriptEncoding::Cp932,
    };
    let mut set = MemoryScripts::new();
    for entry in archive.entries() {
        if !entry.name.to_ascii_lowercase().ends_with(".sc") {
            continue;
        }
        let bytes = archive.read_entry_obj(entry)?;
        let script = parse_script_bytes(entry.name.clone(), &bytes, encoding)?;
        if !script.errors.is_empty() {
            return Err(RuntimeError::Parse(format!(
                "{}:{}: {}",
                entry.name, script.errors[0].line, script.errors[0].message
            )));
        }
        set.insert(script);
    }
    if set.is_empty() {
        return Err(RuntimeError::Parse("scr.paz contains no .sc files".into()));
    }
    Ok(set)
}

fn load_ui_assets(assets: &mut Assets, edition: eden_paz::Edition) -> UiAssets {
    let mut image = |name: &str| assets.image(name).ok();
    let mut ui = UiAssets {
        title: ["topmenu0.png", "topmenu1.png", "topmenu2.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        save_base: ["saveBase0.png", "saveBase1.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        load_base: ["loadBase0.png", "loadBase1.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        music_base: image("musicBase.png"),
        music_pages: ["music1.png", "music2.png", "music3.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        cg_base: ["cgmode0.png", "cgmode1.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        cg_box: ["cgmode0box.png", "cgmode1box.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        cg_menu: ["cgmode0menu.png", "cgmode1menu.png"]
            .iter()
            .filter_map(|n| image(n))
            .collect(),
        cg_mode: if edition == eden_paz::Edition::PlusMosaic { 1 } else { 0 },
        config_base: image("configBase.png"),
        msg_panel: image("msgPanel.png"),
        backlog_gauge: image("backlogGauge.png"),
        icon_auto: image("gameMenuAuto.png"),
        icon_skip: image("gameMenuSkip.png"),
        icon_save: image("gameMenuSave.png"),
        icon_load: image("gameMenuLoad.png"),
        icon_system: image("gameMenuSystem.png"),
        icon_back: image("gameMenuBack.png"),
        select: image("saveloadSelect.png"),
        extra: image("memories.png").or_else(|| image("memscene.png")),
    };
    if ui.save_base.is_empty() {
        ui.save_base = ui.load_base.clone();
    }
    if ui.load_base.is_empty() {
        ui.load_base = ui.save_base.clone();
    }
    ui
}

fn now_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Deterministic UTC breakdown without a chrono dependency.
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, _s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let mut year = 1970i64;
    let mut d = days as i64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0;
    while month < 12 && d >= months[month] {
        d -= months[month];
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        year,
        month + 1,
        d + 1,
        h,
        m
    )
}

/// Sub-pixel advance for one axis of a scroll between two logical ticks.
///
/// `view` is the integer position the VM is currently showing. The result is
/// the fractional offset to add for `alpha` (0..1) of a frame, clamped so it
/// never overshoots the scroll target.
fn scroll_sub(start: i32, target: i32, speed: i32, total_frames: u32, view: i32, alpha: f32) -> f32 {
    if total_frames == 0 || alpha <= 0.0 {
        return 0.0;
    }
    let delta = if speed != 0 {
        speed as f32
    } else {
        (target - start) as f32 / total_frames as f32
    };
    let mut sub = delta * alpha;
    let remaining = (target - view) as f32;
    if delta < 0.0 {
        sub = sub.max(remaining);
    } else if delta > 0.0 {
        sub = sub.min(remaining);
    }
    sub
}

struct VoicePlayback {
    player: AudioPlayer,
    analyzer: LipSyncAnalyzer,
    scratch: Vec<[f32; 2]>,
}

impl VoicePlayback {
    fn new(clip: Arc<AudioClip>, params: PlaybackParams, frame_count: u32) -> Self {
        Self {
            player: AudioPlayer::new(clip, params),
            analyzer: LipSyncAnalyzer::new(frame_count.max(1)),
            scratch: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeStats {
    pub events: u64,
    pub messages: u64,
    pub stages: u64,
    pub transitions: u64,
    pub panels: u64,
    pub movies: u64,
    pub chains: u64,
    pub scrolls: u64,
    pub errors: u64,
}

struct MoviePlayback {
    id: i64,
    decoder: Box<dyn VideoDecoder>,
    audio: Option<AudioPlayer>,
    start_vm_frame: u64,
    target_frame: u64,
    fps: f64,
    finished: bool,
    last_frame: Option<Arc<RgbaImage>>,
}

struct BgmFade {
    from: f32,
    to: f32,
    total: u32,
    elapsed: u32,
    stop_after: bool,
}

/// The complete 60 Hz Musica runtime.
pub struct Runtime {
    game_dir: PathBuf,
    pub assets: Assets,
    provider: Arc<MemoryScripts>,
    vm: Option<ScriptVm>,
    renderer: SoftwareRenderer,
    scene: Scene,
    frame: RgbaImage,
    events: Vec<VmEvent>,
    last_error: Option<String>,
    stage_key: Option<String>,
    stage_ani: Option<Arc<AniFile>>,
    panel_key: Option<String>,
    auto_advance: bool,
    voice: Option<VoicePlayback>,
    se1: Option<AudioPlayer>,
    se2: Option<AudioPlayer>,
    bgm: Option<AudioPlayer>,
    bgm_fade: Option<BgmFade>,
    movie: Option<MoviePlayback>,
    video_enabled: bool,
    ui: UiModel,
    ui_assets: UiAssets,
    ui_frame: RgbaImage,
    ui_visible: bool,
    save_dir: PathBuf,
    unlocked_cgs: BTreeSet<String>,
    unlocked_music: BTreeSet<String>,
    pm_dir: Option<PathBuf>,
    main_dir: PathBuf,
    pm_exe: Option<PathBuf>,
    main_exe: Option<PathBuf>,
    edition: eden_paz::Edition,
    skip_mode: bool,
    quit_requested: bool,
    headless: bool,
    /// Wall-clock remainder between 60 Hz logical ticks (120 Hz display link).
    display_accum: f64,
    stats: RuntimeStats,
}

impl Runtime {
    /// Opens `scr.paz`, `bg.paz`, `sys.paz` and every optional archive in
    /// `game_dir`. `exe` is only needed for archives with an unknown id.
    pub fn open(dir: impl AsRef<Path>, exe: Option<&Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        if !dir.is_dir() {
            return Err(RuntimeError::Io(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
        let nested = dir.join("eden PLUS MOSAIC");
        let pm_dir = if nested.is_dir() {
            Some(nested)
        } else if dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_uppercase().contains("PLUS"))
            .unwrap_or(false)
        {
            Some(dir.clone())
        } else {
            None
        };
        let main_dir = dir.clone();
        let main_exe = exe
            .map(|p| p.to_path_buf())
            .or_else(|| {
                ["eden.exe", "eden_en.exe"]
                    .iter()
                    .map(|n| main_dir.join(n))
                    .find(|p| p.exists())
            });
        let pm_exe = pm_dir.as_ref().and_then(|p| {
            ["eden_pm_en.exe", "eden_pm.exe"]
                .iter()
                .map(|n| p.join(n))
                .find(|p| p.exists())
        });
        let mut assets = Assets::open(&dir, exe)?;
        let edition = assets.scr.edition();
        let provider = Arc::new(load_scripts(&assets.scr)?);
        let renderer = SoftwareRenderer::new(1024, 640).with_system_font();
        let ui_assets = load_ui_assets(&mut assets, edition);
        let save_dir = std::env::temp_dir().join("eden-saves");
        let _ = std::fs::create_dir_all(&save_dir);
        let mut runtime = Self {
            game_dir: dir,
            assets,
            provider,
            vm: None,
            renderer,
            scene: Scene::new(1024, 640),
            frame: RgbaImage::filled(1024, 640, [0, 0, 0, 255]),
            events: Vec::new(),
            last_error: None,
            stage_key: None,
            stage_ani: None,
            panel_key: None,
            auto_advance: false,
            voice: None,
            se1: None,
            se2: None,
            bgm: None,
            bgm_fade: None,
            movie: None,
            video_enabled: true,
            ui: UiModel::default(),
            ui_assets,
            ui_frame: RgbaImage::filled(1024, 640, [0, 0, 0, 255]),
            ui_visible: false,
            save_dir,
            unlocked_cgs: BTreeSet::new(),
            unlocked_music: BTreeSet::new(),
            pm_dir,
            main_dir,
            pm_exe,
            main_exe,
            edition,
            skip_mode: false,
            quit_requested: false,
            headless: false,
            display_accum: 0.0,
            stats: RuntimeStats::default(),
        };
        runtime.load_unlocks();
        runtime.refresh_ui_lists();
        runtime.refresh_slots();
        Ok(runtime)
    }

    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    pub fn edition(&self) -> eden_paz::Edition {
        self.edition
    }

    pub fn set_save_dir(&mut self, dir: impl AsRef<Path>) -> Result<()> {
        self.save_dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&self.save_dir)?;
        self.refresh_slots();
        Ok(())
    }

    /// Headless mode keeps the full VM and scene state machine but skips
    /// decoding stage/panel images. It is used by the 58-script walkthrough;
    /// real pixels are validated by `eden shot` and `eden movie`.
    pub fn stats(&self) -> &RuntimeStats {
        &self.stats
    }

    pub fn reset_stats(&mut self) {
        self.stats = RuntimeStats::default();
    }

    pub fn set_headless(&mut self, enabled: bool) {
        self.headless = enabled;
    }

    pub fn set_video_enabled(&mut self, enabled: bool) {
        self.video_enabled = enabled;
        if !enabled {
            self.stop_movie();
        }
    }

    pub fn video_enabled(&self) -> bool {
        self.video_enabled
    }

    pub fn ui(&self) -> &UiModel {
        &self.ui
    }

    pub fn ui_mut(&mut self) -> &mut UiModel {
        &mut self.ui
    }

    pub fn ui_assets(&self) -> &UiAssets {
        &self.ui_assets
    }

    pub fn ui_frame(&self) -> &RgbaImage {
        &self.ui_frame
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn movie_active(&self) -> bool {
        self.movie.is_some()
    }

    fn load_unlocks(&mut self) {
        let path = self.save_dir.join("unlocks.json");
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        if let Some(items) = value.get("cgs").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(name) = item.as_str() {
                    self.unlocked_cgs.insert(name.to_string());
                }
            }
        }
        if let Some(items) = value.get("music").and_then(|v| v.as_array()) {
            for item in items {
                if let Some(name) = item.as_str() {
                    self.unlocked_music.insert(name.to_string());
                }
            }
        }
    }

    pub fn save_unlocks(&self) {
        let value = serde_json::json!({
            "cgs": self.unlocked_cgs.iter().cloned().collect::<Vec<_>>(),
            "music": self.unlocked_music.iter().cloned().collect::<Vec<_>>(),
        });
        if let Ok(text) = serde_json::to_string_pretty(&value) {
            let _ = std::fs::write(self.save_dir.join("unlocks.json"), text);
        }
    }

    pub fn unlock_cg(&mut self, name: &str) {
        let key = name.trim_start_matches("cgthumb\\").to_string();
        if self
            .assets
            .sys
            .entry(&format!("cgthumb\\{key}"))
            .is_some()
            || self.assets.has(&key)
        {
            self.unlocked_cgs.insert(key);
        }
    }

    pub fn unlock_music(&mut self, file: &str) {
        self.unlocked_music.insert(file.to_string());
    }

    fn step_music(&mut self, delta: i32) {
        let len = self.ui.music.len();
        if len == 0 {
            return;
        }
        let mut index = self.ui.selected_music.unwrap_or(0) as i32;
        for _ in 0..len {
            index = (index + delta).rem_euclid(len as i32);
            let entry = &self.ui.music[index as usize];
            if !entry.unlocked {
                continue;
            }
            let file = entry.file.clone();
            self.ui.selected_music = Some(index as usize);
            self.ui.page = index as usize / 16;
            self.unlock_music(&file);
            if let Err(e) = self.play_bgm(&file, self.ui.config.bgm_volume as i32, 0) {
                self.ui.toast(e.to_string());
            }
            return;
        }
    }

    pub fn refresh_ui_lists(&mut self) {
        let mut gallery = Vec::new();
        for entry in self.assets.sys.entries() {
            let lower = entry.name.to_ascii_lowercase();
            if !lower.starts_with("cgthumb\\") || !lower.ends_with(".png") {
                continue;
            }
            let full_name = entry.name["cgthumb\\".len()..].to_string();
            if !self.assets.has(&full_name) {
                continue;
            }
            gallery.push(GalleryEntry {
                thumb_name: entry.name.clone(),
                unlocked: self.unlocked_cgs.contains(&full_name),
                full_name,
                thumb: None,
                full: None,
            });
        }
        gallery.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        self.ui.gallery = gallery;

        let mut music = Vec::new();
        if let Some(bgm) = &self.assets.bgm {
            for entry in bgm.entries() {
                let lower = entry.name.to_ascii_lowercase();
                if !lower.ends_with(".ogg") {
                    continue;
                }
                let title = entry.name.trim_end_matches(".ogg").to_string();
                music.push(MusicEntry {
                    file: entry.name.clone(),
                    title,
                    unlocked: self.unlocked_music.contains(&entry.name),
                });
            }
        }
        music.sort_by(|a, b| a.file.cmp(&b.file));
        self.ui.music = music;
    }

    fn ensure_gallery_thumbs(&mut self) {
        for i in 0..self.ui.gallery.len() {
            if self.ui.gallery[i].thumb.is_some() {
                continue;
            }
            let name = self.ui.gallery[i].thumb_name.clone();
            if let Ok(image) = self.assets.image(&name) {
                self.ui.gallery[i].thumb = Some(image);
            }
        }
    }

    fn ensure_gallery_full(&mut self, index: usize) {
        let Some(entry) = self.ui.gallery.get(index) else {
            return;
        };
        if entry.full.is_some() || !entry.unlocked {
            return;
        }
        let name = entry.full_name.clone();
        if let Ok(image) = self.assets.image(&name) {
            if let Some(entry) = self.ui.gallery.get_mut(index) {
                entry.full = Some(image);
            }
        }
    }

    pub fn refresh_slots(&mut self) {
        if self.ui.slots.len() < 10 {
            self.ui.slots = (0..10)
                .map(|index| SaveSlot {
                    index,
                    empty: true,
                    ..Default::default()
                })
                .collect();
        }
        for index in 0..10 {
            let state_path = self.save_dir.join(format!("slot_{index:02}.json"));
            let meta_path = self.save_dir.join(format!("slot_{index:02}.meta.json"));
            let thumb_path = self.save_dir.join(format!("slot_{index:02}.png"));
            let mut slot = SaveSlot {
                index,
                empty: true,
                ..Default::default()
            };
            if let Ok(text) = std::fs::read_to_string(&meta_path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    slot.timestamp = value
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    slot.summary = value
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    slot.script = value
                        .get("script")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    slot.empty = false;
                }
            } else if let Ok(text) = std::fs::read_to_string(&state_path) {
                if let Ok(state) = eden_script::SaveState::from_json(&text) {
                    slot.script = state.script;
                    slot.timestamp = "saved".into();
                    slot.summary = state
                        .message
                        .as_ref()
                        .map(|m| m.text.plain_text())
                        .unwrap_or_default();
                    slot.empty = false;
                }
            }
            if !slot.empty {
                if let Ok(bytes) = std::fs::read(&thumb_path) {
                    if let Ok(image) = eden_paz::decode_png(&bytes) {
                        slot.thumbnail = Some(Arc::new(image));
                    }
                }
            }
            self.ui.slots[index] = slot;
        }
    }

    pub fn save_slot(&mut self, index: usize) -> Result<()> {
        if index >= 10 {
            return Err(RuntimeError::Parse(format!("invalid save slot {index}")));
        }
        let (json, script, summary, message_id) = {
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Parse("no script started".into()))?;
            let summary = vm
                .message()
                .map(|m| {
                    let mut text = String::new();
                    if let Some(speaker) = &m.speaker {
                        text.push_str(speaker);
                        text.push_str(": ");
                    }
                    text.push_str(&m.text.plain_text());
                    text.chars().take(80).collect::<String>()
                })
                .unwrap_or_else(|| "(no message)".into());
            (
                vm.save().to_json()?,
                format!("{} @ pc {}", vm.script_name(), vm.pc()),
                summary,
                vm.message().map(|m| m.id).unwrap_or(-1),
            )
        };
        std::fs::write(self.save_dir.join(format!("slot_{index:02}.json")), json)?;
        // The slot thumbnail must show the game stage, never the UI overlay.
        let game_frame = self.renderer.render(&self.scene);
        let png = eden_paz::encode_png(&game_frame)?;
        std::fs::write(self.save_dir.join(format!("slot_{index:02}.png")), png)?;
        let meta = serde_json::json!({
            "timestamp": now_timestamp(),
            "summary": summary,
            "script": script,
            "message_id": message_id,
        });
        std::fs::write(
            self.save_dir.join(format!("slot_{index:02}.meta.json")),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        )?;
        self.save_unlocks();
        self.refresh_slots();
        Ok(())
    }

    pub fn load_slot(&mut self, index: usize) -> Result<()> {
        let path = self.save_dir.join(format!("slot_{index:02}.json"));
        if !path.exists() {
            return Err(RuntimeError::Missing(format!("save slot {index}")));
        }
        self.load_from(path)?;
        self.ui.screen = UiScreen::Game;
        self.ui_visible = false;
        self.ui.hide_ui = false;
        Ok(())
    }

    pub fn latest_slot(&self) -> Option<usize> {
        self.ui
            .slots
            .iter()
            .filter(|s| !s.empty)
            .map(|s| s.index)
            .next_back()
    }

    /// Open a different edition (main ↔ PLUS+MOSAIC) in place.
    pub fn open_edition(&mut self, edition: eden_paz::Edition) -> Result<()> {
        let (dir, exe) = match edition {
            eden_paz::Edition::Main => (self.main_dir.clone(), self.main_exe.clone()),
            eden_paz::Edition::PlusMosaic => {
                let dir = self
                    .pm_dir
                    .clone()
                    .ok_or_else(|| RuntimeError::Missing("eden PLUS MOSAIC directory".into()))?;
                (dir, self.pm_exe.clone())
            }
        };
        self.assets = Assets::open(&dir, exe.as_deref())?;
        self.provider = Arc::new(load_scripts(&self.assets.scr)?);
        self.ui_assets = load_ui_assets(&mut self.assets, edition);
        self.edition = edition;
        self.game_dir = dir;
        self.vm = None;
        self.scene = Scene::new(1024, 640);
        self.stage_key = None;
        self.stage_ani = None;
        self.panel_key = None;
        self.events.clear();
        self.last_error = None;
        self.voice = None;
        self.se1 = None;
        self.se2 = None;
        self.bgm = None;
        self.bgm_fade = None;
        self.stop_movie();
        self.refresh_ui_lists();
        self.refresh_slots();
        Ok(())
    }

    pub fn ui_move(&mut self, x: i32, y: i32) {
        self.ui.move_cursor(x, y);
    }

    pub fn ui_tap(&mut self, x: i32, y: i32) -> bool {
        if let Some(command) = self.ui.tap(x, y) {
            self.handle_ui_command(command);
            self.render_now();
            return true;
        }
        if self.ui.screen == UiScreen::Game && !self.ui.hide_ui {
            self.click();
            self.render_now();
            return true;
        }
        false
    }

    pub fn ui_scroll(&mut self, delta: i32) {
        self.ui.scroll(delta);
        self.render_now();
    }

    pub fn ui_swipe_up(&mut self) {
        if self.ui.screen == UiScreen::Game {
            self.ui.open(UiScreen::Backlog);
            self.ui_visible = true;
            self.render_now();
        }
    }

    pub fn ui_two_finger_tap(&mut self) {
        if self.ui.screen == UiScreen::Game {
            self.ui.hide_ui = !self.ui.hide_ui;
            self.render_now();
        }
    }

    pub fn open_ui(&mut self, screen: UiScreen) {
        if screen == UiScreen::Gallery {
            self.ensure_gallery_thumbs();
        }
        self.ui.open(screen);
        self.ui_visible = screen != UiScreen::Game;
        self.render_now();
    }

    pub fn close_ui(&mut self) {
        self.ui.screen = UiScreen::Game;
        self.ui_visible = false;
        self.render_now();
    }

    pub fn handle_ui_command(&mut self, command: UiCommand) {
        match command {
            UiCommand::StartMain => {
                if self.edition != eden_paz::Edition::Main {
                    if let Err(e) = self.open_edition(eden_paz::Edition::Main) {
                        self.last_error = Some(e.to_string());
                        return;
                    }
                }
                if let Err(e) = self.start_script("A001.sc") {
                    self.last_error = Some(e.to_string());
                }
            }
            UiCommand::StartPlusMosaic => {
                if self.pm_dir.is_none() {
                    self.ui.toast("PLUS+MOSAIC data not found");
                    return;
                }
                if let Err(e) = self.open_edition(eden_paz::Edition::PlusMosaic) {
                    self.last_error = Some(e.to_string());
                    return;
                }
                if let Err(e) = self.start_script("A001.sc") {
                    self.last_error = Some(e.to_string());
                }
            }
            UiCommand::Continue => {
                if let Some(index) = self.latest_slot() {
                    if let Err(e) = self.load_slot(index) {
                        self.ui.toast(e.to_string());
                    }
                } else if let Some(selected) = self.ui.selected_music {
                    if let Some(file) = self.ui.music.get(selected).map(|e| e.file.clone()) {
                        if let Err(e) = self.play_bgm(&file, 100, 0) {
                            self.ui.toast(e.to_string());
                        }
                    }
                } else if let Some(selected) = self.ui.selected_gallery {
                    self.ui.selected_gallery = Some(selected);
                    self.ui.screen = UiScreen::GalleryView;
                } else if self.ui.screen == UiScreen::Music {
                    if let Some(file) = self.ui.music.get(self.ui.page * 16).map(|e| e.file.clone()) {
                        let _ = self.play_bgm(&file, 100, 0);
                    }
                }
            }
            UiCommand::OpenTitle => {
                self.ui.screen = UiScreen::Title;
                self.ui_visible = true;
                self.ui.screen = UiScreen::Title;
            }
            UiCommand::OpenSave => {
                self.ui.open(UiScreen::Save);
                self.ui_visible = true;
            }
            UiCommand::OpenLoad => {
                self.refresh_slots();
                self.ui.open(UiScreen::Load);
                self.ui_visible = true;
            }
            UiCommand::OpenBacklog => {
                self.ui.open(UiScreen::Backlog);
                self.ui_visible = true;
            }
            UiCommand::OpenGallery => {
                self.ensure_gallery_thumbs();
                if self.ui.screen == UiScreen::Gallery {
                    let per_page = 25;
                    if (self.ui.page + 1) * per_page < self.ui.gallery.len() {
                        self.ui.page += 1;
                    }
                } else {
                    self.ui.open(UiScreen::Gallery);
                }
                self.ui_visible = true;
            }
            UiCommand::OpenGalleryView(index) => {
                self.ensure_gallery_full(index);
                self.ui.selected_gallery = Some(index);
                self.ui.screen = UiScreen::GalleryView;
                self.ui_visible = true;
            }
            UiCommand::OpenMusic => {
                if self.ui.screen == UiScreen::Music {
                    let per_page = 16;
                    if (self.ui.page + 1) * per_page < self.ui.music.len() {
                        self.ui.page += 1;
                    }
                } else {
                    self.ui.open(UiScreen::Music);
                }
                self.ui_visible = true;
            }
            UiCommand::PrevPage => {
                self.ui.page = self.ui.page.saturating_sub(1);
            }
            UiCommand::NextPage => {
                let per_page = if self.ui.screen == UiScreen::Gallery {
                    25
                } else {
                    16
                };
                let len = if self.ui.screen == UiScreen::Gallery {
                    self.ui.gallery.len()
                } else {
                    self.ui.music.len()
                };
                if (self.ui.page + 1) * per_page < len {
                    self.ui.page += 1;
                }
            }
            UiCommand::PrevMusic => self.step_music(-1),
            UiCommand::NextMusic => self.step_music(1),
            UiCommand::PlayMovie => {
                if let Err(e) = self.start_script("memOp.sc") {
                    self.ui.toast(e.to_string());
                }
            }
            UiCommand::OpenConfig => {
                self.ui.open(UiScreen::Config);
                self.ui_visible = true;
            }
            UiCommand::OpenExtra => {
                self.ui.open(UiScreen::Extra);
                self.ui_visible = true;
            }
            UiCommand::SaveSlot(index) => match self.save_slot(index) {
                Ok(()) => self.ui.toast(format!("Saved slot {:02}", index + 1)),
                Err(e) => self.ui.toast(e.to_string()),
            },
            UiCommand::LoadSlot(index) => {
                if let Err(e) = self.load_slot(index) {
                    self.ui.toast(e.to_string());
                }
            }
            UiCommand::Back => {
                if self.ui.screen == UiScreen::GalleryView {
                    self.ui.screen = UiScreen::Gallery;
                    self.ui.selected_gallery = None;
                } else {
                    self.ui.back();
                    if self.ui.screen == UiScreen::Game {
                        self.ui_visible = false;
                    }
                }
            }
            UiCommand::PlayMusic(file) => {
                self.unlock_music(&file);
                if let Some(index) = self.ui.music.iter().position(|m| m.file == file) {
                    self.ui.selected_music = Some(index);
                    self.ui.music[index].unlocked = true;
                }
                if let Err(e) = self.play_bgm(&file, self.ui.config.bgm_volume as i32, 0) {
                    self.ui.toast(e.to_string());
                }
                self.save_unlocks();
            }
            UiCommand::StopMusic => {
                self.stop_bgm(0);
            }
            UiCommand::PlayVoice(name) => {
                if let Err(e) = self.play_voice(&name, None) {
                    self.ui.toast(e.to_string());
                }
            }
            UiCommand::SetAuto(_) => {
                self.set_auto_advance(!self.auto_advance);
                self.ui.auto = self.auto_advance;
                self.ui.toast(if self.auto_advance {
                    "Auto mode ON"
                } else {
                    "Auto mode OFF"
                });
            }
            UiCommand::SetSkip(_) => {
                self.skip_mode = !self.skip_mode;
                self.ui.skip = self.skip_mode;
                if self.skip_mode {
                    self.set_auto_advance(true);
                    self.ui.auto = true;
                }
                self.ui.toast(if self.skip_mode { "Skip ON" } else { "Skip OFF" });
            }
            UiCommand::ToggleHideUi => {
                self.ui.hide_ui = !self.ui.hide_ui;
            }
            UiCommand::ConfigChanged => {}
            UiCommand::ConfigAdjust(index, delta) => {
                let apply = |value: &mut u8, delta: i32| {
                    *value = (*value as i32 + delta).clamp(0, 100) as u8;
                };
                match index {
                    0 => apply(&mut self.ui.config.bgm_volume, delta),
                    1 => apply(&mut self.ui.config.se_volume, delta),
                    2 => apply(&mut self.ui.config.voice_volume, delta),
                    3 => apply(&mut self.ui.config.text_speed, delta),
                    4 => apply(&mut self.ui.config.auto_speed, delta),
                    _ => {}
                }
                if let Some(player) = &mut self.bgm {
                    let mut params = player.params();
                    params.volume = self.ui.config.bgm_volume as f32 / 100.0;
                    player.set_params(params);
                }
            }
            UiCommand::Quit => {
                self.quit_requested = true;
            }
        }
    }

    pub fn script_count(&self) -> usize {
        self.provider.len()
    }

    pub fn script_names(&self) -> Vec<String> {
        // MemoryScripts intentionally keeps its map private; the archive is
        // the authoritative ordered list.
        let mut names: Vec<String> = self
            .assets
            .scr
            .entries()
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().ends_with(".sc"))
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    }

    pub fn start_script(&mut self, name: &str) -> Result<()> {
        let mut vm = ScriptVm::new(name, self.provider.clone())?;
        if !vm.script().errors.is_empty() {
            let e = &vm.script().errors[0];
            return Err(RuntimeError::Parse(format!(
                "{}:{}: {}",
                name, e.line, e.message
            )));
        }
        vm.set_auto_advance_clicks(self.auto_advance);
        self.vm = Some(vm);
        self.stop_movie();
        self.ui.screen = UiScreen::Game;
        self.ui_visible = false;
        self.ui.hide_ui = false;
        self.ui.skip = false;
        self.skip_mode = false;
        self.ui.auto = self.auto_advance;
        self.scene = Scene::new(1024, 640);
        self.stage_key = None;
        self.stage_ani = None;
        self.panel_key = None;
        self.events.clear();
        self.last_error = None;
        self.voice = None;
        self.se1 = None;
        self.se2 = None;
        self.bgm = None;
        self.bgm_fade = None;
        // The retail black stage asset, not a synthetic fill.
        if let Ok(black) = self.assets.image("BLACK.png") {
            self.scene.set_stage(black, 0, 0, None);
        }
        self.sync_from_vm()?;
        self.render_now();
        Ok(())
    }

    pub fn vm(&self) -> Option<&ScriptVm> {
        self.vm.as_ref()
    }

    pub fn set_auto_advance(&mut self, enabled: bool) {
        self.auto_advance = enabled;
        if let Some(vm) = &mut self.vm {
            vm.set_auto_advance_clicks(enabled);
        }
    }

    pub fn auto_advance(&self) -> bool {
        self.auto_advance
    }

    pub fn click(&mut self) {
        if self.movie.is_some() {
            self.finish_movie();
            return;
        }
        if let Some(vm) = &mut self.vm {
            vm.click();
        }
    }

    pub fn load_font(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let font = eden_render::FontRenderer::from_path(path)
            .map_err(|e| RuntimeError::Io(format!("font: {e}")))?;
        self.renderer.set_font(font);
        Ok(())
    }

    pub fn use_system_font(&mut self) {
        if let Some(font) = eden_render::FontRenderer::load_system() {
            self.renderer.set_font(font);
        }
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn frame(&self) -> &RgbaImage {
        &self.frame
    }

    pub fn events(&self) -> &[VmEvent] {
        &self.events
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn frame_count(&self) -> u64 {
        self.vm.as_ref().map(|vm| vm.frame()).unwrap_or(0)
    }

    pub fn vm_state(&self) -> Option<&VmState> {
        self.vm.as_ref().map(|vm| vm.state())
    }

    /// Advance exactly one 60 Hz frame and apply the resulting events to the
    /// scene without rasterising. Used by the headless trace tool.
    pub fn tick_vm(&mut self) -> Result<()> {
        self.advance_bgm_fade();
        if self.vm.is_none() {
            // Title / Extra screens are pure UI: no script is running yet.
            self.scene.tick();
            self.ui.tick();
            self.events.clear();
            self.scene.clear_subpixel();
            return Ok(());
        }
        if self.bgm_fade.is_none() {
            if let Some(vm) = &mut self.vm {
                if matches!(vm.state(), VmState::WaitBgm { .. }) {
                    vm.finish_bgm_wait();
                }
            }
        }
        let events = {
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Parse("no script started".into()))?;
            vm.tick().to_vec()
        };
        // Shake/firefly timers advance once per logical frame. The transition
        // timer is authoritative in the VM and is overwritten in sync_from_vm.
        self.scene.tick();
        self.process_events(&events)?;
        self.advance_movie();
        if self.bgm_fade.is_none() {
            if let Some(vm) = &mut self.vm {
                if matches!(vm.state(), VmState::WaitBgm { .. }) {
                    vm.finish_bgm_wait();
                }
            }
        }
        self.events = events;
        self.sync_from_vm()?;
        self.ui.tick();
        self.scene.clear_subpixel();
        Ok(())
    }

    /// Advance by wall-clock time and interpolate the scene for a high
    /// refresh-rate display (ProMotion 120 Hz). Logical state still advances
    /// at exactly 60 Hz; the remainder is applied as a sub-pixel viewport
    /// offset and a sub-frame transition progress so scrolling and wipes stay
    /// smooth instead of stepping at 60 Hz.
    ///
    /// Returns the number of logical 60 Hz frames consumed.
    pub fn tick_display(&mut self, dt_seconds: f64) -> Result<u32> {
        const STEP: f64 = 1.0 / 60.0;
        if !dt_seconds.is_finite() || dt_seconds <= 0.0 {
            self.scene.clear_subpixel();
            return Ok(0);
        }
        // Clamp pathological stalls (debugger, app resume) so the game does
        // not fast-forward through a whole scene in one display callback.
        self.display_accum = (self.display_accum + dt_seconds).min(STEP * 8.0);
        let mut ticks = 0;
        while self.display_accum >= STEP {
            self.tick_vm()?;
            self.display_accum -= STEP;
            ticks += 1;
        }
        let alpha = (self.display_accum / STEP).clamp(0.0, 1.0) as f32;
        self.apply_subpixel(alpha);
        Ok(ticks)
    }

    /// Resets the wall-clock accumulator (for example after the app returns
    /// from the background) so a long pause does not fast-forward the game.
    pub fn reset_display_clock(&mut self) {
        self.display_accum = 0.0;
        self.scene.clear_subpixel();
    }

    fn apply_subpixel(&mut self, alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        let mut sub_x = 0.0f32;
        let mut sub_y = 0.0f32;
        let mut transition_sub = 0.0f32;
        if let Some(vm) = &self.vm {
            match vm.state() {
                VmState::WaitScroll { .. } => {
                    if let Some(scroll) = vm.scroll() {
                        match scroll.kind {
                            ScrollKind::VScroll => {
                                sub_y = scroll_sub(
                                    scroll.start_y,
                                    scroll.target_y,
                                    scroll.speed_y,
                                    scroll.total_frames,
                                    self.scene.view_y,
                                    alpha,
                                );
                            }
                            ScrollKind::HScroll => {
                                sub_x = scroll_sub(
                                    scroll.start_x,
                                    scroll.target_x,
                                    scroll.speed_x,
                                    scroll.total_frames,
                                    self.scene.view_x,
                                    alpha,
                                );
                            }
                            ScrollKind::Scroll => {
                                sub_x = scroll_sub(
                                    scroll.start_x,
                                    scroll.target_x,
                                    scroll.speed_x,
                                    scroll.total_frames,
                                    self.scene.view_x,
                                    alpha,
                                );
                                sub_y = scroll_sub(
                                    scroll.start_y,
                                    scroll.target_y,
                                    scroll.speed_y,
                                    scroll.total_frames,
                                    self.scene.view_y,
                                    alpha,
                                );
                            }
                        }
                    }
                }
                VmState::WaitTransition { .. } => transition_sub = alpha,
                _ => {}
            }
        }
        self.scene.set_view_subpixel(sub_x, sub_y);
        self.scene.set_transition_subpixel(transition_sub);
    }

    /// Advance exactly one 60 Hz frame, apply the resulting events to the
    /// scene, and re-render the 1024×640 framebuffer.
    pub fn tick(&mut self) -> Result<()> {
        let steps = if self.skip_mode { 4 } else { 1 };
        for _ in 0..steps {
            self.tick_vm()?;
        }
        self.render_now();
        Ok(())
    }

    /// Render the current scene without advancing the VM.
    pub fn render_now(&mut self) {
        let base = self.renderer.render(&self.scene);
        if self.ui_visible {
            self.frame = ui::render(
                &self.ui,
                &self.ui_assets,
                &base,
                self.renderer.font.as_ref(),
            );
        } else {
            self.frame = base;
        }
        self.ui_frame = self.frame.clone();
    }

    pub fn render_png(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.render_now();
        let png = eden_paz::encode_png(&self.frame)?;
        std::fs::write(path, png)?;
        Ok(())
    }

    fn process_events(&mut self, events: &[VmEvent]) -> Result<()> {
        for event in events {
            self.stats.events += 1;
            match event {
                VmEvent::Message {
                    id,
                    voice,
                    speaker,
                    text,
                    ..
                } => {
                    self.stats.messages += 1;
                    if self.ui.backlog.last().map(|e| e.id) != Some(*id) {
                        self.ui.backlog.push(eden_render::ui::BacklogEntry {
                            id: *id,
                            speaker: speaker.clone(),
                            text: text.plain_text(),
                            voice: voice.clone(),
                        });
                        if self.ui.backlog.len() > 500 {
                            self.ui.backlog.remove(0);
                        }
                    }
                }
                VmEvent::Transition {
                    kind,
                    layer,
                    frames,
                    ..
                } => {
                    self.stats.transitions += 1;
                    if self.headless {
                        continue;
                    }
                    // Capture the frame that is on screen right now. Stage
                    // events earlier in this batch have already been applied,
                    // so `.stage X` followed by `.transition` wipes *from X*,
                    // while `.transition` followed by `.stage X` wipes from
                    // the previous stage.
                    let from = Arc::new(self.renderer.capture(&self.scene));
                    let mask = match layer.as_deref() {
                        Some(name) if *kind == 1 => Some(self.assets.image(name)?),
                        _ => None,
                    };
                    self.scene.transition = Some(Transition {
                        kind: *kind,
                        mask,
                        total_frames: *frames,
                        elapsed: 0,
                        from: Some(from),
                    });
                }
                VmEvent::TransitionEnd => {
                    self.scene.transition = None;
                }
                VmEvent::ShakeScreen {
                    axis,
                    amplitude,
                    frames,
                    ..
                } => {
                    let kind = match axis {
                        eden_script::ShakeAxis::Random => ShakeKind::Random,
                        eden_script::ShakeAxis::Z => ShakeKind::Sine,
                    };
                    self.scene.start_shake(kind, *amplitude, *frames);
                }
                VmEvent::Effect {
                    kind,
                    count,
                    frames,
                    ..
                } => {
                    if kind.eq_ignore_ascii_case("Firefly") {
                        self.scene.start_firefly(*count, *frames);
                    } else if kind.eq_ignore_ascii_case("end") {
                        self.scene.stop_fireflies();
                    }
                }
                VmEvent::PlayVoice { file, volume, .. } => match file {
                    Some(name) => self.play_voice(name, *volume)?,
                    None => self.voice = None,
                },
                VmEvent::Stage {
                    mask,
                    image,
                    x,
                    y,
                    ..
                } => {
                    self.stats.stages += 1;
                    if !self.headless {
                        // Apply the stage immediately so a `.stage` followed
                        // by `.transition` in the same VM tick captures the
                        // new artwork as the outgoing frame.
                        self.apply_stage_now(mask.clone(), image.clone(), *x, *y)?;
                    }
                }
                VmEvent::Panel { .. } => {
                    self.stats.panels += 1;
                }
                VmEvent::PlayBgm {
                    file,
                    volume,
                    fade_frames,
                    ..
                } => match file {
                    Some(name) => {
                        self.unlock_music(name);
                        self.play_bgm(
                            name,
                            volume.unwrap_or(100),
                            fade_frames.unwrap_or(0),
                        )?
                    }
                    None => self.stop_bgm(fade_frames.unwrap_or(0)),
                },
                VmEvent::PlaySe {
                    channel,
                    file,
                    volume,
                    looping,
                    pan,
                    ..
                } => {
                    let channel = *channel;
                    match file {
                        Some(name) => self.play_se(channel, name, *volume, *looping, *pan)?,
                        None => match channel {
                            1 => self.se1 = None,
                            2 => self.se2 = None,
                            _ => {}
                        },
                    }
                }
                VmEvent::Movie {
                    id,
                    file,
                    width,
                    height,
                    wait,
                    ..
                } => {
                    self.stats.movies += 1;
                    if let Err(e) = self.start_movie(*id, file, *width, *height, *wait) {
                        self.last_error = Some(format!("movie {file}: {e}"));
                        if let Some(vm) = &mut self.vm {
                            vm.finish_movie();
                        }
                    }
                }
                VmEvent::MovieEnd { .. } => {
                    self.stop_movie();
                }
                VmEvent::Chain { .. } => {
                    self.stats.chains += 1;
                }
                VmEvent::Scroll { .. } => {
                    self.stats.scrolls += 1;
                }
                VmEvent::Error { message, line } => {
                    self.stats.errors += 1;
                    self.last_error = Some(format!("line {line}: {message}"));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn apply_stage_now(
        &mut self,
        mask: Option<String>,
        image: String,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let stage_key = format!("{image}|{}|{x}|{y}", mask.as_deref().unwrap_or(""));
        if self.stage_key.as_deref() == Some(&stage_key) {
            return Ok(());
        }
        self.scene
            .set_stage_spec(image.clone(), mask.clone(), x, y);
        let img = self.assets.image(&image)?;
        let mask_img = match mask.as_deref() {
            Some(name) => Some(self.assets.image(name)?),
            None => None,
        };
        self.scene.set_stage(img, x, y, mask_img);
        self.scene.set_view(x, y);
        self.unlock_cg(&image);
        self.stage_ani = self.assets.ani_for_image(&image)?;
        self.stage_key = Some(stage_key);
        Ok(())
    }

    fn sync_from_vm(&mut self) -> Result<()> {
        let (stage, view, panel, message, transition) = {
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Parse("no script started".into()))?;
            (
                vm.stage().clone(),
                vm.view(),
                vm.panel().clone(),
                vm.message().cloned(),
                vm.transition().cloned(),
            )
        };

        let stage_key = format!(
            "{}|{}|{}|{}",
            stage.image,
            stage.mask.as_deref().unwrap_or(""),
            stage.x,
            stage.y
        );
        if self.stage_key.as_deref() != Some(&stage_key) {
            self.scene
                .set_stage_spec(stage.image.clone(), stage.mask.clone(), stage.x, stage.y);
            if self.headless {
                self.stage_ani = None;
            } else {
                let image = self.assets.image(&stage.image)?;
                let mask = match stage.mask.as_deref() {
                    Some(name) => Some(self.assets.image(name)?),
                    None => None,
                };
                self.scene.set_stage(image, stage.x, stage.y, mask);
                self.unlock_cg(&stage.image);
                self.stage_ani = self.assets.ani_for_image(&stage.image)?;
            }
            self.stage_key = Some(stage_key);
        }
        self.scene.set_view(view.x, view.y);

        if let Some(ani) = &self.stage_ani {
            let frame = match &self.voice {
                Some(voice) => {
                    let count = ani.frame_count().max(1) as u32;
                    voice.analyzer.frame().min(count - 1) as usize
                }
                None => ani.closed_frame(),
            };
            self.scene.set_stage_ani(ani.clone(), frame, 255);
        }

        let visible = panel.mode != 0 && !self.ui.hide_ui;
        let overlay = message.map(|m| {
            let mut text = String::new();
            if let Some(speaker) = &m.speaker {
                text.push_str(speaker);
                text.push('\n');
            }
            text.push_str(&m.text.plain_text());
            let shown_chars = text.chars().count();
            MessageOverlay {
                text,
                shown_chars,
                font_size: 24.0,
            }
        });
        self.scene.set_panel(visible, overlay);

        if visible && !self.headless {
            let name = panel.image.as_deref().unwrap_or("msgPanel.png");
            if self.panel_key.as_deref() != Some(name) {
                let image = match self.assets.image(name) {
                    Ok(image) => Some(image),
                    Err(_) => self.assets.image("msgPanel.png").ok(),
                };
                self.scene.set_panel_image(image);
                self.panel_key = Some(name.to_string());
            }
        } else if !self.headless {
            self.scene.set_panel_image(None);
            self.panel_key = None;
        }

        match (&transition, self.scene.transition.as_mut()) {
            (Some(state), Some(scene_transition)) => {
                scene_transition.total_frames = state.total_frames;
                scene_transition.elapsed = state.elapsed;
            }
            _ => self.scene.transition = None,
        }
        Ok(())
    }

    fn start_movie(
        &mut self,
        id: i64,
        file: &str,
        width: u32,
        height: u32,
        wait: bool,
    ) -> Result<()> {
        self.stop_movie();
        if !self.video_enabled {
            if let Some(vm) = &mut self.vm {
                vm.finish_movie();
            }
            return Ok(());
        }
        let path = self.assets.video_file(file)?;
        let avi = AviFile::open(&path)?;
        let width = if width == 0 { avi.video.width } else { width };
        let height = if height == 0 { avi.video.height } else { height };
        let mut decoder = open_decoder(&path, width, height, avi.video.fps)?;
        let audio = match avi.decode_audio()? {
            Some(clip) => Some(AudioPlayer::new(
                Arc::new(clip),
                PlaybackParams::from_percent(
                    self.ui.config.bgm_volume.min(100) as u32,
                    0.0,
                    false,
                ),
            )),
            None => None,
        };
        let first = decoder.next_frame()?;
        let start_vm_frame = self.vm.as_ref().map(|vm| vm.frame()).unwrap_or(0);
        let last_frame = first.map(Arc::new);
        if let Some(frame) = &last_frame {
            self.scene.set_movie(Some(frame.clone()));
        }
        let finished = last_frame.is_none();
        self.movie = Some(MoviePlayback {
            id,
            decoder,
            audio,
            start_vm_frame,
            target_frame: last_frame.is_some() as u64,
            fps: avi.video.fps,
            finished,
            last_frame,
        });
        if wait && finished {
            self.finish_movie();
        }
        Ok(())
    }

    fn advance_movie(&mut self) {
        let Some(movie) = &mut self.movie else {
            return;
        };
        let vm_frame = self
            .vm
            .as_ref()
            .map(|vm| vm.frame())
            .unwrap_or(movie.start_vm_frame);
        let elapsed = vm_frame.saturating_sub(movie.start_vm_frame) as f64 / 60.0;
        let target = (elapsed * movie.fps).floor() as u64;
        let mut error = None;
        while !movie.finished && movie.target_frame <= target {
            match movie.decoder.next_frame() {
                Ok(Some(frame)) => {
                    movie.target_frame += 1;
                    movie.last_frame = Some(Arc::new(frame));
                }
                Ok(None) => {
                    movie.finished = true;
                }
                Err(e) => {
                    error = Some(e.to_string());
                    movie.finished = true;
                }
            }
        }
        if let Some(frame) = &movie.last_frame {
            self.scene.set_movie(Some(frame.clone()));
        }
        let finished = movie.finished;
        let id = movie.id;
        if let Some(error) = error {
            self.last_error = Some(format!("movie decode: {error}"));
        }
        if finished {
            let _ = id;
            self.finish_movie();
        }
    }

    fn finish_movie(&mut self) {
        if self.movie.take().is_some() {
            self.scene.set_movie(None);
            if let Some(vm) = &mut self.vm {
                vm.finish_movie();
            }
        }
    }

    fn stop_movie(&mut self) {
        if self.movie.take().is_some() {
            self.scene.set_movie(None);
        }
    }

    pub fn play_voice(&mut self, name: &str, volume: Option<i32>) -> Result<()> {
        if self.headless {
            return Ok(());
        }
        let archive = self
            .assets
            .voice
            .as_ref()
            .ok_or_else(|| RuntimeError::Missing("voice.paz".into()))?;
        let clip = Arc::new(decode_paz_entry(archive, name)?);
        let frames = self
            .stage_ani
            .as_ref()
            .map(|a| a.frame_count() as u32)
            .unwrap_or(4);
        let params =
            PlaybackParams::from_percent(volume.unwrap_or(100).clamp(0, 100) as u32, 0.0, false);
        self.voice = Some(VoicePlayback::new(clip, params, frames));
        Ok(())
    }

    pub fn stop_voice(&mut self) {
        self.voice = None;
    }

    fn play_bgm(&mut self, name: &str, volume: i32, fade_frames: i32) -> Result<()> {
        if self.headless {
            return Ok(());
        }
        let archive = self
            .assets
            .bgm
            .as_ref()
            .ok_or_else(|| RuntimeError::Missing("bgm.paz".into()))?;
        let clip = Arc::new(decode_paz_entry(archive, name)?);
        let target = (volume.clamp(0, 100) as f32 / 100.0).clamp(0.0, 1.0);
        let fade = fade_frames.max(0) as u32;
        let start = if fade > 0 { 0.0 } else { target };
        let params = PlaybackParams::from_percent((start * 100.0) as u32, 0.0, true);
        self.bgm = Some(AudioPlayer::new(clip, params));
        self.bgm_fade = if fade > 0 {
            Some(BgmFade {
                from: 0.0,
                to: target,
                total: fade,
                elapsed: 0,
                stop_after: false,
            })
        } else {
            None
        };
        Ok(())
    }

    /// `.playBGM * * <frames>` stops the current BGM, optionally fading out.
    fn stop_bgm(&mut self, fade_frames: i32) {
        let fade = fade_frames.max(0) as u32;
        if fade == 0 {
            self.bgm = None;
            self.bgm_fade = None;
            return;
        }
        let from = self.bgm.as_ref().map(|p| p.params().volume).unwrap_or(0.0);
        self.bgm_fade = Some(BgmFade {
            from,
            to: 0.0,
            total: fade,
            elapsed: 0,
            stop_after: true,
        });
    }

    fn advance_bgm_fade(&mut self) {
        let Some(fade) = &mut self.bgm_fade else {
            return;
        };
        fade.elapsed = fade.elapsed.saturating_add(1);
        let t = (fade.elapsed as f32 / fade.total.max(1) as f32).min(1.0);
        let volume = fade.from + (fade.to - fade.from) * t;
        if let Some(player) = &mut self.bgm {
            let mut params = player.params();
            params.volume = volume.clamp(0.0, 1.0);
            player.set_params(params);
        }
        if fade.elapsed >= fade.total {
            let stop = fade.stop_after;
            self.bgm_fade = None;
            if stop {
                self.bgm = None;
            }
        }
    }

    fn play_se(
        &mut self,
        channel: u8,
        name: &str,
        volume: Option<f32>,
        looping: bool,
        pan: f32,
    ) -> Result<()> {
        if self.headless {
            return Ok(());
        }
        let archive = self
            .assets
            .se
            .as_ref()
            .ok_or_else(|| RuntimeError::Missing("se.paz".into()))?;
        let clip = Arc::new(decode_paz_entry(archive, name)?);
        let params = PlaybackParams::from_script(volume, pan, looping);
        let player = AudioPlayer::new(clip, params);
        match channel {
            1 => self.se1 = Some(player),
            2 => self.se2 = Some(player),
            _ => {}
        }
        Ok(())
    }

    /// Mix BGM/SE/Voice into interleaved stereo frames and update the lip-sync
    /// envelope from the voice bus. Returns the number of frames written.
    pub fn mix_audio(&mut self, out: &mut [[f32; 2]]) -> usize {
        for frame in out.iter_mut() {
            *frame = [0.0, 0.0];
        }
        let mut written = 0;
        if let Some(player) = &mut self.bgm {
            written = written.max(player.mix_into(out));
        }
        if let Some(player) = &mut self.se1 {
            written = written.max(player.mix_into(out));
        }
        if let Some(player) = &mut self.se2 {
            written = written.max(player.mix_into(out));
        }
        if let Some(movie) = &mut self.movie {
            if let Some(audio) = &mut movie.audio {
                written = written.max(audio.mix_into(out));
            }
        }
        if let Some(voice) = &mut self.voice {
            voice.scratch.clear();
            voice.scratch.resize(out.len(), [0.0, 0.0]);
            let n = voice.player.mix_into(&mut voice.scratch);
            for i in 0..n {
                out[i][0] += voice.scratch[i][0];
                out[i][1] += voice.scratch[i][1];
            }
            let mut mono = [0.0f32; 512];
            let mut i = 0;
            while i < n {
                let end = (i + mono.len()).min(n);
                for (slot, frame) in mono[..end - i].iter_mut().zip(&voice.scratch[i..end]) {
                    *slot = (frame[0] + frame[1]) * 0.5;
                }
                voice.analyzer.process(&mono[..end - i]);
                i = end;
            }
            written = written.max(n);
        }
        written
    }

    pub fn lipsync_frame(&self) -> u32 {
        self.voice.as_ref().map(|v| v.analyzer.frame()).unwrap_or(0)
    }

    pub fn lipsync_rms(&self) -> f32 {
        self.voice
            .as_ref()
            .map(|v| v.analyzer.envelope())
            .unwrap_or(0.0)
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Parse("no script started".into()))?;
        let json = vm.save().to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_from(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let json = std::fs::read_to_string(path)?;
        let state = eden_script::SaveState::from_json(&json)?;
        let mut vm = ScriptVm::restore(state, self.provider.clone())?;
        vm.set_auto_advance_clicks(self.auto_advance);
        self.vm = Some(vm);
        self.stop_movie();
        self.ui.screen = UiScreen::Game;
        self.ui_visible = false;
        self.stage_key = None;
        self.stage_ani = None;
        self.panel_key = None;
        self.voice = None;
        self.se1 = None;
        self.se2 = None;
        self.bgm = None;
        self.sync_from_vm()?;
        self.render_now();
        Ok(())
    }

    pub fn state_json(&self) -> Result<String> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Parse("no script started".into()))?;
        Ok(vm.save().to_json()?)
    }

    pub fn events_json(&self) -> Result<String> {
        serde_json::to_string(&self.events)
            .map_err(|e| RuntimeError::Parse(format!("events json: {e}")))
    }

    pub fn image(&mut self, name: &str) -> Result<Arc<RgbaImage>> {
        self.assets.image(name)
    }

    pub fn ani(&mut self, name: &str) -> Result<Arc<AniFile>> {
        self.assets.ani(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_dir() -> Option<PathBuf> {
        let path = PathBuf::from("/mnt/f/gal/【PC硬盘】【官中】eden");
        path.exists().then_some(path)
    }

    #[test]
    fn long_stage_ani_and_panel_are_driven_by_a_real_script() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.start_script("A001.sc").unwrap();

        let mut mid_scroll = false;
        let mut text_frame = false;
        for _ in 0..20_000 {
            if matches!(runtime.vm_state(), Some(VmState::WaitClick)) {
                runtime.click();
            }
            runtime.tick_vm().unwrap();
            let (image, view_y, message_id, panel_mode) = {
                let vm = runtime.vm().unwrap();
                (
                    vm.stage().image.clone(),
                    vm.view().y,
                    vm.message().map(|m| m.id),
                    vm.panel().mode,
                )
            };
            if !mid_scroll && image == "ev007a01.png" && (200..600).contains(&view_y) {
                assert!(runtime.stage_ani.is_some(), "ev007a01.ani must be loaded");
                assert!(runtime.stage_ani.as_ref().unwrap().frame_count() == 4);
                runtime.render_now();
                let lit = runtime
                    .frame()
                    .data
                    .chunks_exact(4)
                    .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
                    .count();
                assert!(lit > 1_000, "long stage rendered only {lit} lit pixels");
                mid_scroll = true;
            }
            if mid_scroll && image == "ev007a01.png" && message_id == Some(590) {
                assert!(panel_mode != 0);
                assert_eq!(view_y, 884);
                runtime.render_now();
                assert!(runtime.scene().panel.visible);
                assert!(runtime.scene().panel.image.is_some());
                assert!(runtime.scene().panel.message.is_some());
                text_frame = true;
                break;
            }
        }
        assert!(mid_scroll, "never reached the ev007a01 vscroll");
        assert!(text_frame, "never reached message 590 with the panel");

        let path = std::env::temp_dir().join("eden_runtime_roundtrip.json");
        runtime.save_to(&path).unwrap();
        let before = runtime.state_json().unwrap();
        runtime.load_from(&path).unwrap();
        assert_eq!(before, runtime.state_json().unwrap());
        runtime.render_now();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn bgm_fade_in_and_out_reach_exact_targets() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.play_bgm("edenBGM016.ogg", 100, 10).unwrap();
        assert_eq!(runtime.bgm.as_ref().unwrap().params().volume, 0.0);
        for _ in 0..5 {
            runtime.advance_bgm_fade();
        }
        let mid = runtime.bgm.as_ref().unwrap().params().volume;
        assert!((mid - 0.5).abs() < 0.01, "mid fade volume {mid}");
        for _ in 0..5 {
            runtime.advance_bgm_fade();
        }
        assert!(runtime.bgm_fade.is_none());
        assert!((runtime.bgm.as_ref().unwrap().params().volume - 1.0).abs() < 0.001);
        runtime.stop_bgm(4);
        for _ in 0..4 {
            runtime.advance_bgm_fade();
        }
        assert!(runtime.bgm.is_none());
        assert!(runtime.bgm_fade.is_none());
    }

    #[test]
    fn real_movie_decodes_frames_and_audio() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        if !eden_video::ffmpeg::is_available() {
            eprintln!("skip: ffmpeg missing");
            return;
        }
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.start_script("memOp.sc").unwrap();
        let mut decoded = false;
        let mut audio_written = 0;
        let mut out = vec![[0.0f32; 2]; 4096];
        for _ in 0..180 {
            runtime.tick_vm().unwrap();
            if runtime.movie_active() {
                decoded = true;
            }
            audio_written += runtime.mix_audio(&mut out);
        }
        assert!(decoded, "memOp.sc never opened op.avi");
        runtime.render_now();
        let lit = runtime
            .frame()
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
            .count();
        assert!(lit > 10_000, "movie frame only has {lit} lit pixels");
        assert!(audio_written > 0, "movie PCM track was not mixed");
    }

    #[test]
    fn ui_title_save_load_gallery_render_real_assets() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.open_ui(UiScreen::Title);
        runtime.render_now();
        let title_lit = runtime
            .frame()
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
            .count();
        assert!(title_lit > 100_000);

        runtime.start_script("A001.sc").unwrap();
        for _ in 0..240 {
            runtime.tick_vm().unwrap();
        }
        runtime.save_slot(0).unwrap();
        assert!(!runtime.ui().slots[0].empty);
        runtime.open_ui(UiScreen::Load);
        runtime.render_now();
        assert!(runtime.frame().data.iter().any(|b| *b > 8));

        runtime.open_ui(UiScreen::Gallery);
        runtime.render_now();
        assert!(!runtime.ui().gallery.is_empty());
        assert!(runtime.ui().gallery.iter().any(|g| g.thumb.is_some()));
        runtime.open_ui(UiScreen::Backlog);
        runtime.render_now();
        assert!(!runtime.ui().backlog.is_empty());
    }

    #[test]
    fn real_voice_drives_lipsync() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.start_script("A001.sc").unwrap();
        runtime.play_voice("sio-C013-0183", None).unwrap();

        let mut out = vec![[0.0f32; 2]; 4410];
        let mut peak_frame = 0;
        for _ in 0..12 {
            let written = runtime.mix_audio(&mut out);
            assert!(written > 0);
            peak_frame = peak_frame.max(runtime.lipsync_frame());
            if written < out.len() {
                break;
            }
        }
        assert!(runtime.lipsync_rms() > 0.001);
        assert!(peak_frame >= 2, "mouth never opened: frame {peak_frame}");
    }

    #[test]
    fn title_screen_ticks_without_a_running_script() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.open_ui(UiScreen::Title);
        assert_eq!(runtime.tick_display(1.0 / 60.0).unwrap(), 1);
        runtime.tick_vm().unwrap();
        runtime.render_now();
        let lit = runtime
            .frame()
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
            .count();
        assert!(lit > 100_000, "title screen only has {lit} lit pixels");
        // Tapping the first title item starts A001 through the shared UI.
        assert!(runtime.ui_tap(900, 110));
        assert!(runtime.vm().is_some());
    }

    #[test]
    fn display_tick_interpolates_transitions_and_scrolls() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        let mut runtime = Runtime::open(&dir, None).unwrap();
        runtime.start_script("A001.sc").unwrap();
        runtime.tick_vm().unwrap(); // executes `.transition 0 * 5`
        assert!(matches!(
            runtime.vm_state(),
            Some(VmState::WaitTransition { .. })
        ));
        // A quarter of a 60 Hz frame: the transition is 5 frames long, so the
        // sub-frame progress must be exactly 0.25.
        assert_eq!(runtime.tick_display(1.0 / 240.0).unwrap(), 0);
        assert!((runtime.scene().transition_sub - 0.25).abs() < 1e-6);
        // One full logical frame consumes the remainder.
        assert_eq!(runtime.tick_display(1.0 / 60.0).unwrap(), 1);
        assert!((runtime.scene().transition_sub - 0.25).abs() < 1e-6);
        assert_eq!(runtime.vm().unwrap().transition().unwrap().elapsed, 1);

        // Advance to the long ev006 vertical scroll (stage y=1280, speed -7).
        runtime.set_auto_advance(true);
        runtime.reset_display_clock();
        let mut found = false;
        for _ in 0..20_000 {
            runtime.tick_vm().unwrap();
            let is_ev006_scroll = runtime
                .vm()
                .and_then(|vm| vm.scroll())
                .map(|s| s.start_y == 1280 && s.speed_y == -7)
                .unwrap_or(false);
            if is_ev006_scroll {
                found = true;
                break;
            }
        }
        assert!(found, "A001 never started the ev006 scroll");
        runtime.reset_display_clock();
        let before = runtime.scene().view_y;
        assert_eq!(runtime.tick_display(1.0 / 120.0).unwrap(), 0);
        let scene = runtime.scene();
        assert_eq!(scene.view_y, before);
        assert!(
            (scene.view_sub_y + 3.5).abs() < 1e-6,
            "{}",
            scene.view_sub_y
        );
        // Rendering with the half-frame offset must differ from the integer
        // frame (bilinear sampling actually ran).
        runtime.render_now();
        let half = runtime.frame().clone();
        runtime.tick_display(1.0 / 120.0).unwrap();
        assert_eq!(runtime.scene().view_y, before - 7);
        runtime.render_now();
        assert_ne!(half.data, runtime.frame().data);
    }
}
