//! `eden-script` — Musica `.sc` parser, 60 Hz VM and Serde save states.

pub mod command;
pub mod encoding;
pub mod error;
pub mod parser;
pub mod text;
pub mod vm;

pub use command::{
    CompareOp, EffectCommand, MessageCommand, MovieCommand, PanelCommand, PlayBgmCommand,
    PlaySeCommand, PlayVoiceCommand, ScriptCommand, ShakeAxis, StageCommand, TransitionCommand,
};
pub use encoding::{decode_script, ScriptEncoding};
pub use error::{ParseError, ParseWarning, ScriptError};
pub use parser::{parse_script, parse_script_bytes, Script, ScriptStats};
pub use text::{MessageText, TextSegment};
pub use vm::{
    AudioState, MemoryScripts, MessageState, MovieState, PanelState, SaveState, ScriptProvider,
    ScriptVm, ScrollKind, ScrollState, StackFrame, StageState, TransitionState, ViewState, VmEvent,
    VmState,
};
