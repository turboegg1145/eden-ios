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

use eden_audio::{
    decode_paz_entry, AudioClip, AudioError, AudioPlayer, LipSyncAnalyzer, PlaybackParams,
};
use eden_paz::{AniFile, PazArchive, PazEntry, RgbaImage};
use eden_render::{MessageOverlay, Scene, ShakeKind, SoftwareRenderer, TextureCache, Transition};
use eden_script::{
    parse_script_bytes, MemoryScripts, ScriptEncoding, ScriptError, ScriptVm, VmEvent, VmState,
};

/// Errors surfaced by the runtime.
#[derive(Debug)]
pub enum RuntimeError {
    Paz(eden_paz::PazError),
    Script(ScriptError),
    Audio(AudioError),
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

struct ActiveTransition {
    kind: i32,
    mask: Option<Arc<RgbaImage>>,
    from: RgbaImage,
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
    active_transition: Option<ActiveTransition>,
    stage_key: Option<String>,
    stage_ani: Option<Arc<AniFile>>,
    panel_key: Option<String>,
    auto_advance: bool,
    voice: Option<VoicePlayback>,
    se1: Option<AudioPlayer>,
    se2: Option<AudioPlayer>,
    bgm: Option<AudioPlayer>,
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
        let assets = Assets::open(&dir, exe)?;
        let provider = Arc::new(load_scripts(&assets.scr)?);
        let renderer = SoftwareRenderer::new(1024, 640).with_system_font();
        Ok(Self {
            game_dir: dir,
            assets,
            provider,
            vm: None,
            renderer,
            scene: Scene::new(1024, 640),
            frame: RgbaImage::filled(1024, 640, [0, 0, 0, 255]),
            events: Vec::new(),
            last_error: None,
            active_transition: None,
            stage_key: None,
            stage_ani: None,
            panel_key: None,
            auto_advance: false,
            voice: None,
            se1: None,
            se2: None,
            bgm: None,
        })
    }

    pub fn game_dir(&self) -> &Path {
        &self.game_dir
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
        self.scene = Scene::new(1024, 640);
        self.active_transition = None;
        self.stage_key = None;
        self.stage_ani = None;
        self.panel_key = None;
        self.events.clear();
        self.last_error = None;
        self.voice = None;
        self.se1 = None;
        self.se2 = None;
        self.bgm = None;
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
        self.events = events;
        self.sync_from_vm()?;
        Ok(())
    }

    /// Advance exactly one 60 Hz frame, apply the resulting events to the
    /// scene, and re-render the 1024×640 framebuffer.
    pub fn tick(&mut self) -> Result<()> {
        self.tick_vm()?;
        self.render_now();
        Ok(())
    }

    /// Render the current scene without advancing the VM.
    pub fn render_now(&mut self) {
        self.frame = self.renderer.render(&self.scene);
    }

    pub fn render_png(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.render_now();
        let png = eden_paz::encode_png(&self.frame)?;
        std::fs::write(path, png)?;
        Ok(())
    }

    fn process_events(&mut self, events: &[VmEvent]) -> Result<()> {
        for event in events {
            match event {
                VmEvent::Transition {
                    kind,
                    layer,
                    frames: _,
                    ..
                } => {
                    // Capture the outgoing frame before the following `.stage`
                    // event replaces the stage layer.
                    let from = self.renderer.capture(&self.scene);
                    let mask = match layer.as_deref() {
                        Some(name) if *kind == 1 => Some(self.assets.image(name)?),
                        _ => None,
                    };
                    self.active_transition = Some(ActiveTransition {
                        kind: *kind,
                        mask,
                        from,
                    });
                }
                VmEvent::TransitionEnd => {
                    self.active_transition = None;
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
                VmEvent::PlayBgm { file, volume, .. } => match file {
                    Some(name) => self.play_bgm(name, volume.unwrap_or(100))?,
                    None => self.bgm = None,
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
                VmEvent::Error { message, line } => {
                    self.last_error = Some(format!("line {line}: {message}"));
                }
                _ => {}
            }
        }
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
            let image = self.assets.image(&stage.image)?;
            let mask = match stage.mask.as_deref() {
                Some(name) => Some(self.assets.image(name)?),
                None => None,
            };
            self.scene
                .set_stage_spec(stage.image.clone(), stage.mask.clone(), stage.x, stage.y);
            self.scene.set_stage(image, stage.x, stage.y, mask);
            self.stage_ani = self.assets.ani_for_image(&stage.image)?;
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

        let visible = panel.mode != 0;
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

        if visible {
            let name = panel.image.as_deref().unwrap_or("msgPanel.png");
            if self.panel_key.as_deref() != Some(name) {
                let image = match self.assets.image(name) {
                    Ok(image) => Some(image),
                    Err(_) => self.assets.image("msgPanel.png").ok(),
                };
                self.scene.set_panel_image(image);
                self.panel_key = Some(name.to_string());
            }
        } else {
            self.scene.set_panel_image(None);
            self.panel_key = None;
        }

        match (&transition, &self.active_transition) {
            (Some(state), Some(active)) => {
                if let Some(scene_transition) = &mut self.scene.transition {
                    scene_transition.kind = active.kind;
                    scene_transition.mask = active.mask.clone();
                    scene_transition.total_frames = state.total_frames;
                    scene_transition.elapsed = state.elapsed;
                } else {
                    self.scene.transition = Some(Transition {
                        kind: active.kind,
                        mask: active.mask.clone(),
                        total_frames: state.total_frames,
                        elapsed: state.elapsed,
                        from: Some(active.from.clone()),
                    });
                }
            }
            _ => {
                self.scene.transition = None;
                if transition.is_none() {
                    self.active_transition = None;
                }
            }
        }
        Ok(())
    }

    pub fn play_voice(&mut self, name: &str, volume: Option<i32>) -> Result<()> {
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

    fn play_bgm(&mut self, name: &str, volume: i32) -> Result<()> {
        let archive = self
            .assets
            .bgm
            .as_ref()
            .ok_or_else(|| RuntimeError::Missing("bgm.paz".into()))?;
        let clip = Arc::new(decode_paz_entry(archive, name)?);
        let params = PlaybackParams::from_percent(volume.clamp(0, 100) as u32, 0.0, true);
        self.bgm = Some(AudioPlayer::new(clip, params));
        Ok(())
    }

    fn play_se(
        &mut self,
        channel: u8,
        name: &str,
        volume: Option<f32>,
        looping: bool,
        pan: f32,
    ) -> Result<()> {
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
        self.active_transition = None;
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
        runtime.set_auto_advance(true);
        runtime.start_script("A001.sc").unwrap();

        let mut mid_scroll = false;
        let mut text_frame = false;
        for _ in 0..8000 {
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
}
