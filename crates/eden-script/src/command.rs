//! Typed representation of the 24 commands actually used by eden*, plus a
//! compatibility branch for the other 23 Musica commands.

use serde::{Deserialize, Serialize};

use crate::text::MessageText;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageCommand {
    pub id: i64,
    pub voice: Option<String>,
    /// Speaker shown on the name plate (`亮`, `Maya`, ...), when present.
    pub speaker: Option<String>,
    pub text: MessageText,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCommand {
    /// `None` for `*`.
    pub mask: Option<String>,
    pub image: String,
    pub x: i32,
    pub y: i32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionCommand {
    /// 0 = cross-fade, 1 = rule-mask.
    pub kind: i32,
    /// `None` for `*`.
    pub layer: Option<String>,
    pub frames: u32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelCommand {
    /// 0 = hide, 1 = show, 2 = special.
    pub mode: i32,
    pub mask: Option<String>,
    pub image: Option<String>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayBgmCommand {
    /// `None` for `*` (stop/fade out).
    pub file: Option<String>,
    pub volume: Option<i32>,
    pub fade_frames: Option<i32>,
    pub raw_args: Vec<String>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaySeCommand {
    /// `None` for `*` (stop the channel).
    pub file: Option<String>,
    /// `[n]` or the first value of `[a,b]`, as a fraction 0.0..=1.0.
    pub volume: Option<f32>,
    /// Raw `[a,b]` pair when present (`hiro_reizouko_open.ogg[100,100]`).
    pub volume_pair: Option<(i32, i32)>,
    pub looping: bool,
    /// -1.0 (left) ..= 1.0 (right).
    pub pan: f32,
    /// The fourth argument, usually `*`.
    pub reserved: Option<String>,
    pub raw_args: Vec<String>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayVoiceCommand {
    pub file: Option<String>,
    pub volume: Option<i32>,
    pub raw_args: Vec<String>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitCommand {
    pub frames: u32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PragmaCommand {
    pub disable_control: bool,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelCommand {
    pub name: String,
    pub line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CompareOp {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "==" => Some(CompareOp::Eq),
            "!=" => Some(CompareOp::Ne),
            "<" => Some(CompareOp::Lt),
            "<=" => Some(CompareOp::Le),
            ">" => Some(CompareOp::Gt),
            ">=" => Some(CompareOp::Ge),
            _ => None,
        }
    }

    pub fn eval(self, a: i64, b: i64) -> bool {
        match self {
            CompareOp::Eq => a == b,
            CompareOp::Ne => a != b,
            CompareOp::Lt => a < b,
            CompareOp::Le => a <= b,
            CompareOp::Gt => a > b,
            CompareOp::Ge => a >= b,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfCommand {
    pub var: String,
    pub op: CompareOp,
    pub value: i64,
    pub label: String,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndScrollCommand {
    /// `t` or `f`.
    pub flag: bool,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainCommand {
    pub file: String,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetVarCommand {
    pub var: String,
    pub value: i64,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VScrollCommand {
    pub target: i32,
    pub speed: i32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HScrollCommand {
    pub target: i32,
    pub speed: i32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollCommand {
    pub target_x: i32,
    pub target_y: i32,
    pub frames: u32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovieCommand {
    pub id: i64,
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub wait: bool,
    pub line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShakeAxis {
    /// Random jitter (`R`).
    Random,
    /// Deterministic sine (`Z`).
    Z,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShakeCommand {
    pub axis: ShakeAxis,
    pub amplitude: i32,
    pub frames: u32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectCommand {
    Firefly {
        count: u32,
        frames: u32,
        line: usize,
    },
    End {
        line: usize,
    },
    Other {
        name: String,
        args: Vec<String>,
        line: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GotoCommand {
    pub label: String,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatCommand {
    pub name: String,
    pub args: Vec<String>,
    /// True for the 23 documented Musica commands; false for anything else.
    pub known: bool,
    pub line: usize,
}

/// One parsed script line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScriptCommand {
    Message(MessageCommand),
    Stage(StageCommand),
    Transition(TransitionCommand),
    Panel(PanelCommand),
    PlayBgm(PlayBgmCommand),
    PlaySe(PlaySeCommand),
    PlaySe2(PlaySeCommand),
    PlayVoice(PlayVoiceCommand),
    Wait(WaitCommand),
    Pragma(PragmaCommand),
    Label(LabelCommand),
    If(IfCommand),
    EndScroll(EndScrollCommand),
    Chain(ChainCommand),
    SetGlobal(SetVarCommand),
    VScroll(VScrollCommand),
    End { line: usize },
    Set(SetVarCommand),
    Scroll(ScrollCommand),
    Movie(MovieCommand),
    HScroll(HScrollCommand),
    Goto(GotoCommand),
    ShakeScreen(ShakeCommand),
    Effect(EffectCommand),
    Compat(CompatCommand),
}

impl ScriptCommand {
    pub fn name(&self) -> &str {
        match self {
            ScriptCommand::Message(_) => ".message",
            ScriptCommand::Stage(_) => ".stage",
            ScriptCommand::Transition(_) => ".transition",
            ScriptCommand::Panel(_) => ".panel",
            ScriptCommand::PlayBgm(_) => ".playBGM",
            ScriptCommand::PlaySe(_) => ".playSE",
            ScriptCommand::PlaySe2(_) => ".playSE2",
            ScriptCommand::PlayVoice(_) => ".playVoice",
            ScriptCommand::Wait(_) => ".wait",
            ScriptCommand::Pragma(_) => ".pragma",
            ScriptCommand::Label(_) => ".label",
            ScriptCommand::If(_) => ".if",
            ScriptCommand::EndScroll(_) => ".endScroll",
            ScriptCommand::Chain(_) => ".chain",
            ScriptCommand::SetGlobal(_) => ".setGlobal",
            ScriptCommand::VScroll(_) => ".vscroll",
            ScriptCommand::End { .. } => ".end",
            ScriptCommand::Set(_) => ".set",
            ScriptCommand::Scroll(_) => ".scroll",
            ScriptCommand::Movie(_) => ".movie",
            ScriptCommand::HScroll(_) => ".hscroll",
            ScriptCommand::Goto(_) => ".goto",
            ScriptCommand::ShakeScreen(_) => ".shakeScreen",
            ScriptCommand::Effect(_) => ".effect",
            ScriptCommand::Compat(c) => &c.name,
        }
    }

    pub fn line(&self) -> usize {
        match self {
            ScriptCommand::Message(c) => c.line,
            ScriptCommand::Stage(c) => c.line,
            ScriptCommand::Transition(c) => c.line,
            ScriptCommand::Panel(c) => c.line,
            ScriptCommand::PlayBgm(c) => c.line,
            ScriptCommand::PlaySe(c) | ScriptCommand::PlaySe2(c) => c.line,
            ScriptCommand::PlayVoice(c) => c.line,
            ScriptCommand::Wait(c) => c.line,
            ScriptCommand::Pragma(c) => c.line,
            ScriptCommand::Label(c) => c.line,
            ScriptCommand::If(c) => c.line,
            ScriptCommand::EndScroll(c) => c.line,
            ScriptCommand::Chain(c) => c.line,
            ScriptCommand::SetGlobal(c) | ScriptCommand::Set(c) => c.line,
            ScriptCommand::VScroll(c) => c.line,
            ScriptCommand::End { line } => *line,
            ScriptCommand::Scroll(c) => c.line,
            ScriptCommand::Movie(c) => c.line,
            ScriptCommand::HScroll(c) => c.line,
            ScriptCommand::Goto(c) => c.line,
            ScriptCommand::ShakeScreen(c) => c.line,
            ScriptCommand::Effect(c) => match c {
                EffectCommand::Firefly { line, .. }
                | EffectCommand::End { line }
                | EffectCommand::Other { line, .. } => *line,
            },
            ScriptCommand::Compat(c) => c.line,
        }
    }
}

/// The 23 Musica commands that eden* never executes. They are parsed into
/// [`ScriptCommand::Compat`] so a future script cannot crash the VM.
pub const COMPAT_COMMANDS: &[&str] = &[
    ".char",
    ".animation",
    ".overlay",
    ".screen",
    ".setFG",
    ".cutin",
    ".HSVfilter",
    ".rotateBG",
    ".rotateFG",
    ".frame",
    ".select",
    ".movie2",
    ".exec",
    ".debug",
    "#include",
    ".setTimer",
    ".waitTimer",
    ".waitAnimation",
    ".waitBGM",
    ".deleteVar",
    ".scrollFG",
    ".scrollAcc",
    ".wscroll",
];

pub fn is_compat_command(name: &str) -> bool {
    COMPAT_COMMANDS.iter().any(|c| *c == name)
}
