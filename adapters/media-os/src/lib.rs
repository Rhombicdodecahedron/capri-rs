//! The `media` tool: music control that works with any player, through the system's media keys.
//!
//! - Windows: the same virtual keys a keyboard's media keys send (Spotify, browsers, Deezer and
//!   most players obey them), and `spotify:` links opened through the shell.
//! - Linux: `playerctl` (MPRIS) and `pactl`; links with `xdg-open`.
//! - macOS: links with `open`; media keys are not wired yet.

use async_trait::async_trait;
use capri_application::ports::{Tool, ToolError, ToolSpec};
use capri_application::services::MEDIA_TOOL;
use capri_domain::{MediaCommand, Risk};

#[cfg(windows)]
mod windows;

#[derive(Debug, Default, Clone, Copy)]
pub struct MediaKeys;

/// A `spotify:` link that opens the desktop app on a search.
pub fn spotify_search_uri(query: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("spotify:search:");
    for b in query.trim().bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn run(cmd: &MediaCommand) -> Result<(), String> {
    match cmd {
        MediaCommand::OpenSpotify => open("spotify:"),
        MediaCommand::Search(q) => open(&spotify_search_uri(q)),
        other => key(other),
    }
}

#[cfg(windows)]
fn key(cmd: &MediaCommand) -> Result<(), String> {
    windows::media_key(cmd)
}

#[cfg(windows)]
fn open(uri: &str) -> Result<(), String> {
    windows::open(uri)
}

#[cfg(not(windows))]
fn key(cmd: &MediaCommand) -> Result<(), String> {
    let (program, args): (&str, &[&str]) = match cmd {
        MediaCommand::PlayPause => ("playerctl", &["play-pause"]),
        MediaCommand::Play => ("playerctl", &["play"]),
        MediaCommand::Pause => ("playerctl", &["pause"]),
        MediaCommand::Next => ("playerctl", &["next"]),
        MediaCommand::Previous => ("playerctl", &["previous"]),
        MediaCommand::VolumeUp => ("pactl", &["set-sink-volume", "@DEFAULT_SINK@", "+8%"]),
        MediaCommand::VolumeDown => ("pactl", &["set-sink-volume", "@DEFAULT_SINK@", "-8%"]),
        MediaCommand::Mute => ("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"]),
        MediaCommand::OpenSpotify | MediaCommand::Search(_) => unreachable!("handled by open"),
    };
    let status = std::process::Command::new(program).args(args).status().map_err(|e| format!("{program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{program} failed ({status})")) }
}

#[cfg(not(windows))]
fn open(uri: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program).arg(uri).spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
}

#[async_trait]
impl Tool for MediaKeys {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: MEDIA_TOOL.into(),
            description: "Control music playback: play, pause, next, previous, volume, open Spotify, search Spotify."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["play_pause", "play", "pause", "next", "previous", "volume_up", "volume_down", "mute", "open_spotify", "search"] },
                    "query": { "type": "string", "description": "what to look up, for action=search" }
                },
                "required": ["action"]
            }),
            risk: Risk::Reversible,
        }
    }

    async fn run(&self, args: serde_json::Value) -> Result<String, ToolError> {
        let cmd: MediaCommand = serde_json::from_value(args).map_err(|e| ToolError::BadArguments(e.to_string()))?;
        tracing::info!(?cmd, "media");
        run(&cmd).map(|()| "done".into()).map_err(ToolError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_links_are_encoded() {
        assert_eq!(spotify_search_uri("daft punk"), "spotify:search:daft%20punk");
        assert_eq!(spotify_search_uri("Beyoncé"), "spotify:search:Beyonc%C3%A9");
    }

    #[test]
    fn arguments_match_the_domain_command() {
        let args = serde_json::to_value(MediaCommand::Search("x".into())).unwrap();
        assert_eq!(args, serde_json::json!({"action": "search", "query": "x"}));
        let back: MediaCommand = serde_json::from_value(serde_json::json!({"action": "next"})).unwrap();
        assert_eq!(back, MediaCommand::Next);
    }
}
