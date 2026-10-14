//! Core modules of the Rust Encoder desktop application.
//! FFmpeg and FFprobe remain external media tools; app logic is Rust.
pub mod encode;
pub mod media;
pub mod model;
pub mod player;
pub mod preview_stream;
pub mod audio_preview;
pub mod dependencies;
pub mod hardware;
