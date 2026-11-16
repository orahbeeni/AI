//! Cutline's engine: everything that doesn't draw. The window lives in `main.rs` and `ui/`.
//!
//! Adding an editing feature later (trims, filters, titles...) means teaching `plan` to produce
//! the right FFmpeg arguments; `export` runs whatever the plan says and `ui` only edits settings.

pub mod export;
pub mod log;
pub mod media;
pub mod plan;
pub mod preview;
pub mod tools;
