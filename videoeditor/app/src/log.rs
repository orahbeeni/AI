//! Lines shown in the console.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogKind {
    /// A command as it starts.
    Command,
    /// Something Cutline itself says.
    Info,
    /// FFmpeg's own output.
    Output,
    Warning,
    Error,
    Success,
    /// A running status line, replaced by the next one.
    Live,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

impl LogLine {
    pub fn new(kind: LogKind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into() }
    }
}
