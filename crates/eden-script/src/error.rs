//! Error and warning types for script parsing and VM execution.

use std::fmt;

/// A syntax error at a specific physical line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub source: String,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: {} — {:?}",
            self.line, self.message, self.source
        )
    }
}

/// A non-fatal parser warning (unknown command, unusual argument shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    pub line: usize,
    pub source: String,
    pub message: String,
}

/// A VM-level error (missing label, missing chained script, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    ScriptNotFound(String),
    LabelNotFound { script: String, label: String },
    InvalidPc { script: String, pc: usize },
    InvalidSave(String),
    ChainLoop(String),
    Io(String),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::ScriptNotFound(s) => write!(f, "script not found: {s}"),
            ScriptError::LabelNotFound { script, label } => {
                write!(f, "label {label:?} not found in {script}")
            }
            ScriptError::InvalidPc { script, pc } => {
                write!(f, "pc {pc} is outside {script}")
            }
            ScriptError::InvalidSave(s) => write!(f, "invalid save state: {s}"),
            ScriptError::ChainLoop(s) => write!(f, "chain loop or depth exceeded at {s}"),
            ScriptError::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for ScriptError {}

impl From<std::io::Error> for ScriptError {
    fn from(e: std::io::Error) -> Self {
        ScriptError::Io(e.to_string())
    }
}
