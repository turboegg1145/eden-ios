//! Fixed-60 Hz script VM, event stream and Serde save states.
//!
//! Every timing parameter in a `.sc` file is expressed in frames. The VM
//! advances exactly one logical frame per [`ScriptVm::tick`], so `.wait 180`
//! is always 3.000 s and `.transition ... 30` is always 0.500 s, regardless of
//! the display refresh rate.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::command::*;
use crate::encoding::ScriptEncoding;
use crate::error::ScriptError;
use crate::parser::{parse_script_bytes, Script};
use crate::text::MessageText;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewState {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageState {
    pub mask: Option<String>,
    pub image: String,
    pub x: i32,
    pub y: i32,
}

impl Default for StageState {
    fn default() -> Self {
        Self {
            mask: None,
            image: "BLACK.png".into(),
            x: 0,
            y: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelState {
    pub mode: i32,
    pub mask: Option<String>,
    pub image: Option<String>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            mode: 0,
            mask: None,
            image: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioState {
    pub file: String,
    pub volume: i32,
    pub fade_frames: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageState {
    pub id: i64,
    pub voice: Option<String>,
    pub speaker: Option<String>,
    pub text: MessageText,
    pub shown_chars: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackFrame {
    pub script: String,
    pub pc: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollKind {
    VScroll,
    HScroll,
    Scroll,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollState {
    pub kind: ScrollKind,
    pub start_x: i32,
    pub start_y: i32,
    pub target_x: i32,
    pub target_y: i32,
    pub speed_x: i32,
    pub speed_y: i32,
    pub total_frames: u32,
    pub elapsed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionState {
    pub kind: i32,
    pub layer: Option<String>,
    pub total_frames: u32,
    pub elapsed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovieState {
    pub id: i64,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub wait: bool,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmState {
    Running,
    WaitFrames { remaining: u32 },
    WaitClick,
    WaitScroll { remaining: u32 },
    WaitTransition { remaining: u32, total: u32 },
    WaitMovie { id: i64 },
    Ended,
    /// Waits for the active BGM fade (or a fixed frame count).
    WaitBgm { remaining: Option<u32> },
}

/// Events emitted for the presentation/audio layers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum VmEvent {
    Message {
        id: i64,
        voice: Option<String>,
        speaker: Option<String>,
        text: MessageText,
        line: usize,
    },
    Stage {
        mask: Option<String>,
        image: String,
        x: i32,
        y: i32,
        line: usize,
    },
    Transition {
        kind: i32,
        layer: Option<String>,
        frames: u32,
        line: usize,
    },
    Panel {
        mode: i32,
        mask: Option<String>,
        image: Option<String>,
        line: usize,
    },
    PlayBgm {
        file: Option<String>,
        volume: Option<i32>,
        fade_frames: Option<i32>,
        line: usize,
    },
    PlaySe {
        channel: u8,
        file: Option<String>,
        volume: Option<f32>,
        looping: bool,
        pan: f32,
        line: usize,
    },
    PlayVoice {
        file: Option<String>,
        volume: Option<i32>,
        line: usize,
    },
    Wait {
        frames: u32,
        line: usize,
    },
    Scroll {
        kind: ScrollKind,
        start_x: i32,
        start_y: i32,
        target_x: i32,
        target_y: i32,
        total_frames: u32,
        line: usize,
    },
    EndScroll {
        flag: bool,
        line: usize,
    },
    Movie {
        id: i64,
        file: String,
        width: u32,
        height: u32,
        wait: bool,
        line: usize,
    },
    ShakeScreen {
        axis: ShakeAxis,
        amplitude: i32,
        frames: u32,
        line: usize,
    },
    Effect {
        kind: String,
        count: u32,
        frames: u32,
        line: usize,
    },
    Chain {
        from: String,
        to: String,
        line: usize,
    },
    Goto {
        label: String,
        line: usize,
    },
    Set {
        var: String,
        value: i64,
        global: bool,
        line: usize,
    },
    Pragma {
        disable_control: bool,
        line: usize,
    },
    TransitionEnd,
    ScrollEnd,
    MovieEnd {
        id: i64,
    },
    End {
        line: usize,
    },
    Compat {
        name: String,
        args: Vec<String>,
        line: usize,
    },
    Error {
        message: String,
        line: usize,
    },
}

/// Source of chained scripts.
pub trait ScriptProvider: Send + Sync {
    fn load(&self, name: &str) -> Option<Arc<Script>>;
}

/// In-memory script set used by the CLI and tests.
#[derive(Default, Clone)]
pub struct MemoryScripts {
    scripts: HashMap<String, Arc<Script>>,
}

impl MemoryScripts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, script: Script) {
        let name = script.name.to_lowercase();
        self.scripts.insert(name, Arc::new(script));
    }

    pub fn insert_arc(&mut self, name: &str, script: Arc<Script>) {
        self.scripts.insert(name.to_lowercase(), script);
    }

    pub fn from_dir(dir: impl AsRef<Path>, encoding: ScriptEncoding) -> std::io::Result<Self> {
        let mut set = Self::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sc") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let bytes = fs::read(&path)?;
            match parse_script_bytes(name.clone(), &bytes, encoding) {
                Ok(script) => set.insert(script),
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{name}: {e}"),
                    ))
                }
            }
        }
        Ok(set)
    }

    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }
}

impl ScriptProvider for MemoryScripts {
    fn load(&self, name: &str) -> Option<Arc<Script>> {
        self.scripts.get(&name.to_lowercase()).cloned()
    }
}

/// The 60 Hz script state machine.
pub struct ScriptVm {
    script: Arc<Script>,
    script_name: String,
    pc: usize,
    call_stack: Vec<StackFrame>,
    vars: BTreeMap<String, i64>,
    globals: BTreeMap<String, i64>,
    state: VmState,
    frame: u64,
    view: ViewState,
    stage: StageState,
    panel: PanelState,
    bgm: Option<AudioState>,
    message: Option<MessageState>,
    scroll: Option<ScrollState>,
    transition: Option<TransitionState>,
    movie: Option<MovieState>,
    pragma_control: bool,
    auto_advance_clicks: bool,
    click_advance: bool,
    bgm_wait_done: bool,
    events: Vec<VmEvent>,
    provider: Arc<dyn ScriptProvider>,
    max_commands_per_tick: usize,
    last_error: Option<ScriptError>,
}

impl ScriptVm {
    pub fn new(script_name: &str, provider: Arc<dyn ScriptProvider>) -> Result<Self, ScriptError> {
        let script = provider
            .load(script_name)
            .ok_or_else(|| ScriptError::ScriptNotFound(script_name.to_string()))?;
        Ok(Self::from_script(script, provider))
    }

    pub fn from_script(script: Arc<Script>, provider: Arc<dyn ScriptProvider>) -> Self {
        let script_name = script.name.clone();
        Self {
            script,
            script_name,
            pc: 0,
            call_stack: Vec::new(),
            vars: BTreeMap::new(),
            globals: BTreeMap::new(),
            state: VmState::Running,
            frame: 0,
            view: ViewState::default(),
            stage: StageState::default(),
            panel: PanelState::default(),
            bgm: None,
            message: None,
            scroll: None,
            transition: None,
            movie: None,
            pragma_control: true,
            auto_advance_clicks: false,
            click_advance: false,
            bgm_wait_done: false,
            events: Vec::new(),
            provider,
            max_commands_per_tick: 4096,
            last_error: None,
        }
    }

    pub fn script_name(&self) -> &str {
        &self.script_name
    }

    pub fn script(&self) -> &Arc<Script> {
        &self.script
    }

    pub fn pc(&self) -> usize {
        self.pc
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn state(&self) -> &VmState {
        &self.state
    }

    pub fn view(&self) -> ViewState {
        self.view
    }

    pub fn stage(&self) -> &StageState {
        &self.stage
    }

    pub fn panel(&self) -> &PanelState {
        &self.panel
    }

    pub fn bgm(&self) -> Option<&AudioState> {
        self.bgm.as_ref()
    }

    pub fn message(&self) -> Option<&MessageState> {
        self.message.as_ref()
    }

    pub fn scroll(&self) -> Option<&ScrollState> {
        self.scroll.as_ref()
    }

    pub fn transition(&self) -> Option<&TransitionState> {
        self.transition.as_ref()
    }

    pub fn movie(&self) -> Option<&MovieState> {
        self.movie.as_ref()
    }

    pub fn vars(&self) -> &BTreeMap<String, i64> {
        &self.vars
    }

    pub fn globals(&self) -> &BTreeMap<String, i64> {
        &self.globals
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: i64) {
        self.globals.insert(name.into(), value);
    }

    pub fn set_var(&mut self, name: impl Into<String>, value: i64) {
        self.vars.insert(name.into(), value);
    }

    pub fn var(&self, name: &str) -> i64 {
        self.vars
            .get(name)
            .or_else(|| self.globals.get(name))
            .copied()
            .unwrap_or(0)
    }

    pub fn set_auto_advance_clicks(&mut self, enabled: bool) {
        self.auto_advance_clicks = enabled;
    }

    pub fn auto_advance_clicks(&self) -> bool {
        self.auto_advance_clicks
    }

    pub fn click(&mut self) {
        self.click_advance = true;
    }

    pub fn finish_movie(&mut self) {
        if let Some(m) = &mut self.movie {
            m.done = true;
        }
    }

    /// Called by the runtime once the active BGM fade has completed.
    pub fn finish_bgm_wait(&mut self) {
        self.bgm_wait_done = true;
    }

    pub fn last_error(&self) -> Option<&ScriptError> {
        self.last_error.as_ref()
    }

    pub fn events(&self) -> &[VmEvent] {
        &self.events
    }

    /// Advance exactly one 60 Hz frame and return the events produced.
    pub fn tick(&mut self) -> &[VmEvent] {
        self.events.clear();
        self.frame += 1;
        self.advance_state();
        if matches!(self.state, VmState::Running) {
            self.run_until_wait();
        }
        &self.events
    }

    fn advance_state(&mut self) {
        match &mut self.state {
            VmState::Running | VmState::Ended => {}
            VmState::WaitFrames { remaining } => {
                if *remaining > 0 {
                    *remaining -= 1;
                }
                if *remaining == 0 {
                    self.state = VmState::Running;
                }
            }
            VmState::WaitClick => {
                if self.click_advance || self.auto_advance_clicks {
                    self.click_advance = false;
                    self.state = VmState::Running;
                }
            }
            VmState::WaitScroll { remaining } => {
                if *remaining > 0 {
                    *remaining -= 1;
                }
                let done = if let Some(s) = &mut self.scroll {
                    s.elapsed = s.elapsed.saturating_add(1);
                    s.elapsed >= s.total_frames
                } else {
                    true
                };
                if let Some(s) = &self.scroll {
                    self.view = scroll_position(s);
                }
                if done {
                    if let Some(s) = self.scroll.take() {
                        self.view = ViewState {
                            x: s.target_x,
                            y: s.target_y,
                        };
                    }
                    self.state = VmState::Running;
                    self.events.push(VmEvent::ScrollEnd);
                }
            }
            VmState::WaitTransition { remaining, .. } => {
                if *remaining > 0 {
                    *remaining -= 1;
                }
                if let Some(t) = &mut self.transition {
                    t.elapsed = t.elapsed.saturating_add(1);
                }
                if *remaining == 0 {
                    self.transition = None;
                    self.state = VmState::Running;
                    self.events.push(VmEvent::TransitionEnd);
                }
            }
            VmState::WaitBgm { remaining } => {
                let done = match remaining {
                    Some(left) => {
                        if *left > 0 {
                            *left -= 1;
                        }
                        *left == 0
                    }
                    None => self.bgm_wait_done,
                };
                if done {
                    self.bgm_wait_done = false;
                    self.state = VmState::Running;
                }
            }
            VmState::WaitMovie { id } => {
                let id = *id;
                let done = self.movie.as_ref().map(|m| m.done).unwrap_or(true);
                // Auto-advance is for dialogue only. A `.movie ... t` wait must
                // run to the decoder's end unless the player explicitly taps to
                // skip, otherwise enabling auto mode would skip every movie.
                if done || self.click_advance {
                    self.click_advance = false;
                    self.movie = None;
                    self.state = VmState::Running;
                    self.events.push(VmEvent::MovieEnd { id });
                }
            }
        }
    }

    fn run_until_wait(&mut self) {
        let mut budget = self.max_commands_per_tick;
        while matches!(self.state, VmState::Running) {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if self.pc >= self.script.commands.len() {
                self.state = VmState::Ended;
                self.events.push(VmEvent::End {
                    line: self.script.commands.last().map(|c| c.line()).unwrap_or(0),
                });
                break;
            }
            let cmd = self.script.commands[self.pc].clone();
            let before_script = self.script_name.clone();
            self.pc += 1;
            self.execute(cmd);
            // Yield when `.chain` or `.end` switches scripts so the trace and
            // save system can observe the transition at a frame boundary.
            if self.script_name != before_script {
                break;
            }
        }
    }

    fn execute(&mut self, cmd: ScriptCommand) {
        match cmd {
            ScriptCommand::Message(m) => {
                self.message = Some(MessageState {
                    id: m.id,
                    voice: m.voice.clone(),
                    speaker: m.speaker.clone(),
                    text: m.text.clone(),
                    shown_chars: 0,
                });
                self.events.push(VmEvent::Message {
                    id: m.id,
                    voice: m.voice,
                    speaker: m.speaker,
                    text: m.text.clone(),
                    line: m.line,
                });
                // Every retail `.message` is a text box the player advances;
                // `\a` marks lines that auto mode may skip after their voice,
                // not lines that wait while unmarked narration does not.
                if !self.auto_advance_clicks {
                    self.state = VmState::WaitClick;
                }
            }
            ScriptCommand::Stage(s) => {
                self.stage = StageState {
                    mask: s.mask.clone(),
                    image: s.image.clone(),
                    x: s.x,
                    y: s.y,
                };
                // The stage's x/y are the initial viewport offset into the
                // artwork (`.stage * ev006.png 0 1280` starts at the bottom of
                // a tall image). `.vscroll 970 -7` continues from there.
                self.view = ViewState { x: s.x, y: s.y };
                self.events.push(VmEvent::Stage {
                    mask: s.mask,
                    image: s.image,
                    x: s.x,
                    y: s.y,
                    line: s.line,
                });
                // A pending `.transition` starts its timer on the stage change.
                if let Some(t) = &self.transition {
                    if t.total_frames > 0 {
                        self.state = VmState::WaitTransition {
                            remaining: t.total_frames,
                            total: t.total_frames,
                        };
                    }
                }
            }
            ScriptCommand::Transition(t) => {
                self.transition = Some(TransitionState {
                    kind: t.kind,
                    layer: t.layer.clone(),
                    total_frames: t.frames,
                    elapsed: 0,
                });
                self.events.push(VmEvent::Transition {
                    kind: t.kind,
                    layer: t.layer,
                    frames: t.frames,
                    line: t.line,
                });
                // `.transition` only arms the wipe: the following `.stage`
                // supplies the incoming artwork and starts the timer. The
                // runtime captures the outgoing frame when it sees this event,
                // before the stage is applied.
            }
            ScriptCommand::Panel(p) => {
                self.panel = PanelState {
                    mode: p.mode,
                    mask: p.mask.clone(),
                    image: p.image.clone(),
                };
                self.events.push(VmEvent::Panel {
                    mode: p.mode,
                    mask: p.mask,
                    image: p.image,
                    line: p.line,
                });
            }
            ScriptCommand::PlayBgm(b) => {
                self.bgm = b.file.as_ref().map(|f| AudioState {
                    file: f.clone(),
                    volume: b.volume.unwrap_or(100),
                    fade_frames: b.fade_frames.unwrap_or(0),
                });
                self.events.push(VmEvent::PlayBgm {
                    file: b.file,
                    volume: b.volume,
                    fade_frames: b.fade_frames,
                    line: b.line,
                });
            }
            ScriptCommand::PlaySe(s) => {
                self.events.push(VmEvent::PlaySe {
                    channel: 1,
                    file: s.file,
                    volume: s.volume,
                    looping: s.looping,
                    pan: s.pan,
                    line: s.line,
                });
            }
            ScriptCommand::PlaySe2(s) => {
                self.events.push(VmEvent::PlaySe {
                    channel: 2,
                    file: s.file,
                    volume: s.volume,
                    looping: s.looping,
                    pan: s.pan,
                    line: s.line,
                });
            }
            ScriptCommand::PlayVoice(v) => {
                self.events.push(VmEvent::PlayVoice {
                    file: v.file,
                    volume: v.volume,
                    line: v.line,
                });
            }
            ScriptCommand::Wait(w) => {
                self.events.push(VmEvent::Wait {
                    frames: w.frames,
                    line: w.line,
                });
                if w.frames > 0 {
                    self.state = VmState::WaitFrames {
                        remaining: w.frames,
                    };
                }
            }
            ScriptCommand::WaitBgm(w) => {
                self.bgm_wait_done = false;
                match w.frames {
                    Some(0) => {}
                    Some(frames) => {
                        self.state = VmState::WaitBgm {
                            remaining: Some(frames),
                        }
                    }
                    None => self.state = VmState::WaitBgm { remaining: None },
                }
            }
            ScriptCommand::Pragma(p) => {
                self.pragma_control = !p.disable_control;
                self.events.push(VmEvent::Pragma {
                    disable_control: p.disable_control,
                    line: p.line,
                });
            }
            ScriptCommand::Label(_) => {}
            ScriptCommand::If(i) => {
                let value = self.var(&i.var);
                if i.op.eval(value, i.value) {
                    self.jump_to(&i.label, i.line);
                }
            }
            ScriptCommand::EndScroll(e) => {
                self.scroll = None;
                if matches!(self.state, VmState::WaitScroll { .. }) {
                    self.state = VmState::Running;
                }
                self.events.push(VmEvent::EndScroll {
                    flag: e.flag,
                    line: e.line,
                });
            }
            ScriptCommand::Chain(c) => {
                if self.call_stack.len() >= 64 {
                    self.fail(ScriptError::ChainLoop(c.file.clone()), c.line);
                    return;
                }
                let Some(next) = self.provider.load(&c.file) else {
                    self.fail(ScriptError::ScriptNotFound(c.file.clone()), c.line);
                    return;
                };
                self.call_stack.push(StackFrame {
                    script: self.script_name.clone(),
                    pc: self.pc,
                });
                self.events.push(VmEvent::Chain {
                    from: self.script_name.clone(),
                    to: c.file.clone(),
                    line: c.line,
                });
                self.script = next;
                self.script_name = c.file;
                self.pc = 0;
            }
            ScriptCommand::SetGlobal(s) => {
                self.globals.insert(s.var.clone(), s.value);
                self.events.push(VmEvent::Set {
                    var: s.var,
                    value: s.value,
                    global: true,
                    line: s.line,
                });
            }
            ScriptCommand::Set(s) => {
                self.vars.insert(s.var.clone(), s.value);
                self.events.push(VmEvent::Set {
                    var: s.var,
                    value: s.value,
                    global: false,
                    line: s.line,
                });
            }
            ScriptCommand::VScroll(v) => {
                self.start_scroll(
                    ScrollKind::VScroll,
                    self.view.x,
                    v.target,
                    0,
                    v.speed,
                    0,
                    v.line,
                );
            }
            ScriptCommand::HScroll(h) => {
                self.start_scroll(
                    ScrollKind::HScroll,
                    h.target,
                    self.view.y,
                    h.speed,
                    0,
                    0,
                    h.line,
                );
            }
            ScriptCommand::Scroll(s) => {
                self.start_scroll(
                    ScrollKind::Scroll,
                    s.target_x,
                    s.target_y,
                    0,
                    0,
                    s.frames,
                    s.line,
                );
            }
            ScriptCommand::End { line } => {
                if let Some(frame) = self.call_stack.pop() {
                    if let Some(script) = self.provider.load(&frame.script) {
                        self.script = script;
                        self.script_name = frame.script;
                        self.pc = frame.pc;
                    } else {
                        self.fail(ScriptError::ScriptNotFound(frame.script), line);
                    }
                } else {
                    self.state = VmState::Ended;
                    self.events.push(VmEvent::End { line });
                }
            }
            ScriptCommand::Movie(m) => {
                self.movie = Some(MovieState {
                    id: m.id,
                    file: m.file.clone(),
                    width: m.width,
                    height: m.height,
                    wait: m.wait,
                    done: false,
                });
                self.events.push(VmEvent::Movie {
                    id: m.id,
                    file: m.file,
                    width: m.width,
                    height: m.height,
                    wait: m.wait,
                    line: m.line,
                });
                if m.wait {
                    self.state = VmState::WaitMovie { id: m.id };
                }
            }
            ScriptCommand::Goto(g) => {
                self.events.push(VmEvent::Goto {
                    label: g.label.clone(),
                    line: g.line,
                });
                self.jump_to(&g.label, g.line);
            }
            ScriptCommand::ShakeScreen(s) => {
                self.events.push(VmEvent::ShakeScreen {
                    axis: s.axis,
                    amplitude: s.amplitude,
                    frames: s.frames,
                    line: s.line,
                });
            }
            ScriptCommand::Effect(e) => {
                let (kind, count, frames, line) = match e {
                    EffectCommand::Firefly {
                        count,
                        frames,
                        line,
                    } => ("Firefly".to_string(), count, frames, line),
                    EffectCommand::End { line } => ("end".to_string(), 0, 0, line),
                    EffectCommand::Other { name, line, .. } => (name, 0, 0, line),
                };
                self.events.push(VmEvent::Effect {
                    kind,
                    count,
                    frames,
                    line,
                });
            }
            ScriptCommand::Compat(c) => {
                self.events.push(VmEvent::Compat {
                    name: c.name,
                    args: c.args,
                    line: c.line,
                });
            }
        }
    }

    fn start_scroll(
        &mut self,
        kind: ScrollKind,
        target_x: i32,
        target_y: i32,
        speed_x: i32,
        speed_y: i32,
        frames: u32,
        line: usize,
    ) {
        // Scrolls continue from wherever the viewport currently is. The first
        // scroll after `.stage` therefore starts at the stage's x/y (`.stage`
        // resets the view there), while a second `.vscroll` picks up from the
        // position the previous one reached instead of jumping back.
        let start_x = self.view.x;
        let start_y = self.view.y;
        self.view = ViewState {
            x: start_x,
            y: start_y,
        };
        let total_frames = match kind {
            ScrollKind::VScroll => {
                if speed_y == 0 {
                    0
                } else {
                    ((target_y - start_y).abs() as f32 / speed_y.abs() as f32).ceil() as u32
                }
            }
            ScrollKind::HScroll => {
                if speed_x == 0 {
                    0
                } else {
                    ((target_x - start_x).abs() as f32 / speed_x.abs() as f32).ceil() as u32
                }
            }
            ScrollKind::Scroll => frames,
        };
        let s = ScrollState {
            kind,
            start_x,
            start_y,
            target_x,
            target_y,
            speed_x,
            speed_y,
            total_frames,
            elapsed: 0,
        };
        self.events.push(VmEvent::Scroll {
            kind,
            start_x,
            start_y,
            target_x,
            target_y,
            total_frames,
            line,
        });
        if total_frames == 0 {
            self.view = ViewState {
                x: target_x,
                y: target_y,
            };
            self.scroll = None;
            return;
        }
        self.scroll = Some(s);
        self.state = VmState::WaitScroll {
            remaining: total_frames,
        };
    }

    fn jump_to(&mut self, label: &str, line: usize) {
        match self.script.label(label) {
            Some(pc) => self.pc = pc,
            None => self.fail(
                ScriptError::LabelNotFound {
                    script: self.script_name.clone(),
                    label: label.to_string(),
                },
                line,
            ),
        }
    }

    fn fail(&mut self, error: ScriptError, line: usize) {
        self.events.push(VmEvent::Error {
            message: error.to_string(),
            line,
        });
        self.last_error = Some(error);
        self.state = VmState::Ended;
    }

    /// Capture a serialisable snapshot.
    pub fn save(&self) -> SaveState {
        SaveState {
            version: SaveState::VERSION,
            script: self.script_name.clone(),
            pc: self.pc,
            call_stack: self.call_stack.clone(),
            vars: self.vars.clone(),
            globals: self.globals.clone(),
            state: self.state.clone(),
            frame: self.frame,
            view: self.view,
            stage: self.stage.clone(),
            panel: self.panel.clone(),
            bgm: self.bgm.clone(),
            message: self.message.clone(),
            scroll: self.scroll.clone(),
            transition: self.transition.clone(),
            movie: self.movie.clone(),
            pragma_control: self.pragma_control,
        }
    }

    /// Restore a snapshot. `provider` must be able to load the saved script.
    pub fn restore(
        state: SaveState,
        provider: Arc<dyn ScriptProvider>,
    ) -> Result<Self, ScriptError> {
        if state.version != SaveState::VERSION {
            return Err(ScriptError::InvalidSave(format!(
                "unsupported save version {}",
                state.version
            )));
        }
        let script = provider
            .load(&state.script)
            .ok_or_else(|| ScriptError::ScriptNotFound(state.script.clone()))?;
        if state.pc > script.commands.len() {
            return Err(ScriptError::InvalidPc {
                script: state.script,
                pc: state.pc,
            });
        }
        Ok(Self {
            script,
            script_name: state.script,
            pc: state.pc,
            call_stack: state.call_stack,
            vars: state.vars,
            globals: state.globals,
            state: state.state,
            frame: state.frame,
            view: state.view,
            stage: state.stage,
            panel: state.panel,
            bgm: state.bgm,
            message: state.message,
            scroll: state.scroll,
            transition: state.transition,
            movie: state.movie,
            pragma_control: state.pragma_control,
            auto_advance_clicks: false,
            click_advance: false,
            bgm_wait_done: false,
            events: Vec::new(),
            provider,
            max_commands_per_tick: 4096,
            last_error: None,
        })
    }
}

fn scroll_position(s: &ScrollState) -> ViewState {
    let e = s.elapsed.min(s.total_frames);
    match s.kind {
        ScrollKind::VScroll => {
            let mut y = s.start_y.saturating_add(s.speed_y.saturating_mul(e as i32));
            if s.speed_y < 0 {
                y = y.max(s.target_y);
            } else if s.speed_y > 0 {
                y = y.min(s.target_y);
            }
            ViewState { x: s.start_x, y }
        }
        ScrollKind::HScroll => {
            let mut x = s.start_x.saturating_add(s.speed_x.saturating_mul(e as i32));
            if s.speed_x < 0 {
                x = x.max(s.target_x);
            } else if s.speed_x > 0 {
                x = x.min(s.target_x);
            }
            ViewState { x, y: s.start_y }
        }
        ScrollKind::Scroll => {
            if s.total_frames == 0 {
                return ViewState {
                    x: s.target_x,
                    y: s.target_y,
                };
            }
            let t = e as f64 / s.total_frames as f64;
            ViewState {
                x: s.start_x + ((s.target_x - s.start_x) as f64 * t).round() as i32,
                y: s.start_y + ((s.target_y - s.start_y) as f64 * t).round() as i32,
            }
        }
    }
}

/// Versioned, Serde-compatible save snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveState {
    pub version: u32,
    pub script: String,
    pub pc: usize,
    pub call_stack: Vec<StackFrame>,
    pub vars: BTreeMap<String, i64>,
    pub globals: BTreeMap<String, i64>,
    pub state: VmState,
    pub frame: u64,
    pub view: ViewState,
    pub stage: StageState,
    pub panel: PanelState,
    pub bgm: Option<AudioState>,
    pub message: Option<MessageState>,
    pub scroll: Option<ScrollState>,
    pub transition: Option<TransitionState>,
    pub movie: Option<MovieState>,
    pub pragma_control: bool,
}

impl SaveState {
    pub const VERSION: u32 = 1;

    pub fn to_json(&self) -> Result<String, ScriptError> {
        serde_json::to_string_pretty(self).map_err(|e| ScriptError::InvalidSave(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, ScriptError> {
        serde_json::from_str(s).map_err(|e| ScriptError::InvalidSave(e.to_string()))
    }

    pub fn to_binary(&self) -> Result<Vec<u8>, ScriptError> {
        bincode::serialize(self).map_err(|e| ScriptError::InvalidSave(e.to_string()))
    }

    pub fn from_binary(bytes: &[u8]) -> Result<Self, ScriptError> {
        bincode::deserialize(bytes).map_err(|e| ScriptError::InvalidSave(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(scripts: &[(&str, &str)]) -> Arc<MemoryScripts> {
        let mut set = MemoryScripts::new();
        for (name, text) in scripts {
            let script = crate::parse_script(*name, text);
            assert!(script.errors.is_empty(), "{:?}", script.errors);
            set.insert(script);
        }
        Arc::new(set)
    }

    #[test]
    fn wait_180_is_exactly_three_seconds() {
        let p = provider(&[("t.sc", ".wait 180\n.message 1 #done\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.set_auto_advance_clicks(true);
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitFrames { remaining: 180 });
        for _ in 0..179 {
            vm.tick();
        }
        assert_eq!(vm.state(), &VmState::WaitFrames { remaining: 1 });
        vm.tick();
        // The next command runs on the frame the counter reaches zero.
        assert!(vm.message().is_some());
        assert_eq!(vm.frame(), 181);
    }

    #[test]
    fn message_wait_click_needs_input() {
        let p = provider(&[("t.sc", ".message 1 #hi\\a\n.message 2 #bye\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitClick);
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitClick);
        vm.click();
        vm.tick();
        assert_eq!(vm.message().unwrap().id, 2);
    }

    #[test]
    fn auto_advance_skips_click() {
        let p = provider(&[("t.sc", ".message 1 #hi\\a\n.message 2 #bye\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.set_auto_advance_clicks(true);
        vm.tick();
        assert_eq!(vm.message().unwrap().id, 2);
    }

    #[test]
    fn vscroll_uses_speed_and_clamps() {
        let p = provider(&[(
            "t.sc",
            ".stage * bg.png 0 1280\n.vscroll 970 -7\n.endScroll f\n",
        )]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick(); // stage + vscroll start
        assert_eq!(vm.view().y, 1280);
        assert_eq!(vm.scroll().unwrap().total_frames, 45);
        vm.tick();
        assert_eq!(vm.view().y, 1273);
        for _ in 0..44 {
            vm.tick();
        }
        assert_eq!(vm.view().y, 970);
        // `.endScroll f` runs on the same frame the scroll finishes.
        assert!(vm.scroll().is_none());
        assert!(matches!(vm.state(), VmState::Ended));
    }

    #[test]
    fn consecutive_vscrolls_continue_from_the_current_view() {
        let p = provider(&[(
            "t.sc",
            ".stage * bg.png 0 1280\n.vscroll 970 -7\n.endScroll f\n.vscroll 0 -10\n.endScroll f\n",
        )]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick(); // stage + first vscroll
        assert_eq!(vm.view().y, 1280);
        for _ in 0..45 {
            vm.tick();
        }
        assert_eq!(vm.view().y, 970);
        assert_eq!(vm.scroll().unwrap().start_y, 970);
        assert_eq!(vm.scroll().unwrap().total_frames, 97);
        vm.tick();
        assert_eq!(vm.view().y, 960);
    }

    #[test]
    fn transition_runs_for_exact_frames() {
        let p = provider(&[("t.sc", ".transition 0 * 30\n.stage * bg.png 0 0\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        // `.transition` arms the wipe; `.stage` starts the timer.
        assert_eq!(
            vm.state(),
            &VmState::WaitTransition {
                remaining: 30,
                total: 30
            }
        );
        let events = vm.events();
        assert!(matches!(events.first(), Some(VmEvent::Transition { .. })));
        assert!(matches!(events.get(1), Some(VmEvent::Stage { .. })));
        for _ in 0..29 {
            vm.tick();
        }
        assert!(matches!(vm.state(), VmState::WaitTransition { .. }));
        vm.tick();
        assert!(vm.transition().is_none());
        assert!(matches!(vm.state(), VmState::Ended));
    }

    #[test]
    fn transition_without_a_stage_does_not_block() {
        let p = provider(&[("t.sc", ".transition 0 * 30\n.message 1 #hi\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        // The message waits for the player, not for the unused transition.
        assert!(matches!(vm.state(), VmState::WaitClick));
        assert!(vm.transition().is_some());
    }

    #[test]
    fn chain_and_end_pop_the_stack() {
        let p = provider(&[
            ("a.sc", ".chain b.sc\n.set x = 2\n.end\n"),
            ("b.sc", ".set y = 1\n.end\n"),
        ]);
        let mut vm = ScriptVm::new("a.sc", p).unwrap();
        vm.tick();
        assert_eq!(vm.script_name(), "b.sc");
        vm.tick();
        assert_eq!(vm.script_name(), "a.sc");
        assert_eq!(vm.var("y"), 1);
        vm.tick();
        assert_eq!(vm.var("x"), 2);
        assert!(matches!(vm.state(), VmState::Ended));
    }

    #[test]
    fn if_and_goto() {
        let p = provider(&[(
            "t.sc",
            ".set x = 1\n.if x == 1 yes\n.set y = 9\n.goto done\n.label yes\n.set y = 7\n.label done\n.end\n",
        )]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        assert_eq!(vm.var("y"), 7);
        assert!(matches!(vm.state(), VmState::Ended));
    }

    #[test]
    fn movie_wait_finishes_on_signal() {
        let p = provider(&[("t.sc", ".movie 1 op.avi 1024 640 t\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitMovie { id: 1 });
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitMovie { id: 1 });
        vm.finish_movie();
        vm.tick();
        assert!(vm.movie().is_none());
        assert!(matches!(vm.state(), VmState::Ended));
    }

    #[test]
    fn movie_wait_ignores_auto_advance_but_accepts_skip_click() {
        let p = provider(&[("t.sc", ".movie 1 op.avi 1024 640 t\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.set_auto_advance_clicks(true);
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitMovie { id: 1 });
        for _ in 0..10 {
            vm.tick();
        }
        assert_eq!(vm.state(), &VmState::WaitMovie { id: 1 });
        vm.click();
        vm.tick();
        assert!(matches!(vm.state(), VmState::Ended));
        assert!(vm.movie().is_none());
    }

    #[test]
    fn wait_bgm_waits_for_runtime_signal_or_frames() {
        let p = provider(&[("t.sc", ".playBGM x.ogg\n.waitBGM\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitBgm { remaining: None });
        for _ in 0..5 {
            vm.tick();
        }
        assert_eq!(vm.state(), &VmState::WaitBgm { remaining: None });
        vm.finish_bgm_wait();
        vm.tick();
        assert!(matches!(vm.state(), VmState::Ended));

        let p = provider(&[("t.sc", ".waitBGM 3\n.end\n")]);
        let mut vm = ScriptVm::new("t.sc", p).unwrap();
        vm.tick();
        assert_eq!(vm.state(), &VmState::WaitBgm { remaining: Some(3) });
        for _ in 0..2 {
            vm.tick();
        }
        assert_eq!(vm.state(), &VmState::WaitBgm { remaining: Some(1) });
        vm.tick();
        assert!(matches!(vm.state(), VmState::Ended));
    }

    #[test]
    fn save_json_and_binary_roundtrip() {
        let p = provider(&[(
            "t.sc",
            ".set x = 5\n.stage * bg.png 0 1280\n.vscroll 970 -7\n.message 1 #hi\\a\n.end\n",
        )]);
        let mut vm = ScriptVm::new("t.sc", p.clone()).unwrap();
        vm.set_auto_advance_clicks(true);
        for _ in 0..20 {
            vm.tick();
        }
        let save = vm.save();
        let json = save.to_json().unwrap();
        let from_json = SaveState::from_json(&json).unwrap();
        assert_eq!(save, from_json);
        let bin = save.to_binary().unwrap();
        let from_bin = SaveState::from_binary(&bin).unwrap();
        assert_eq!(save, from_bin);

        let mut a = ScriptVm::restore(from_json, p.clone()).unwrap();
        let mut b = ScriptVm::restore(from_bin, p).unwrap();
        a.set_auto_advance_clicks(true);
        b.set_auto_advance_clicks(true);
        for _ in 0..100 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.save(), b.save());
        assert!(a.last_error().is_none());
    }

    #[test]
    fn restore_rejects_bad_pc() {
        let p = provider(&[("t.sc", ".wait 1\n")]);
        let mut save = ScriptVm::new("t.sc", p.clone()).unwrap().save();
        save.pc = 999;
        assert!(ScriptVm::restore(save, p).is_err());
    }
}
