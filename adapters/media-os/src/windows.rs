//! Media keys and links on Windows.
#![allow(unsafe_code)]

use capri_domain::MediaCommand;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
    VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn key_event(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let flags = KEYEVENTF_EXTENDEDKEY | if up { KEYEVENTF_KEYUP } else { 0 };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

pub fn media_key(cmd: &MediaCommand) -> Result<(), String> {
    // Windows has a single play/pause toggle key: play and pause both press it
    let vk = match cmd {
        MediaCommand::PlayPause | MediaCommand::Play | MediaCommand::Pause => VK_MEDIA_PLAY_PAUSE,
        MediaCommand::Next => VK_MEDIA_NEXT_TRACK,
        MediaCommand::Previous => VK_MEDIA_PREV_TRACK,
        MediaCommand::VolumeUp => VK_VOLUME_UP,
        MediaCommand::VolumeDown => VK_VOLUME_DOWN,
        MediaCommand::Mute => VK_VOLUME_MUTE,
        MediaCommand::OpenSpotify
        | MediaCommand::Search(_)
        | MediaCommand::Shuffle
        | MediaCommand::Repeat
        | MediaCommand::Like
        | MediaCommand::NowPlaying => return Ok(()),
    };
    // volume keys move by 2%: press them a few times so it is noticeable
    let presses = if matches!(cmd, MediaCommand::VolumeUp | MediaCommand::VolumeDown) { 4 } else { 1 };
    let mut inputs = Vec::with_capacity(presses * 2);
    for _ in 0..presses {
        inputs.push(key_event(vk, false));
        inputs.push(key_event(vk, true));
    }
    // SAFETY: a slice of initialised INPUT structs and its exact length and element size.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() { Ok(()) } else { Err("Windows refused the media key".into()) }
}

pub fn open(uri: &str) -> Result<(), String> {
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, target) = (wide("open"), wide(uri));
    // SAFETY: null-terminated UTF-16 strings that outlive the call.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // values above 32 mean success
    if r as isize > 32 { Ok(()) } else { Err(format!("could not open {uri}: is Spotify installed?")) }
}
