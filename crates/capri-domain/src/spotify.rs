//! Spotify, driven like a user would: the buttons of the desktop app by their accessible names (in
//! the languages Spotify ships), with its keyboard shortcuts as a fallback. No Web API, no account
//! keys, Free accounts included.

use serde::{Deserialize, Serialize};

use crate::ui::{Key, KeyChord, Locator, Modifier, Role, WindowInfo};
use crate::words::normalize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "query", rename_all = "snake_case")]
pub enum SpotifyAction {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Shuffle,
    Repeat,
    Like,
    /// Search and start the best match.
    PlayQuery(String),
    /// What is playing, from the window title.
    NowPlaying,
    Open,
}

/// A button to press, and the shortcut that does the same when the button can't be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    pub button: Locator,
    pub shortcut: Option<KeyChord>,
}

const PLAY: &[&str] = &["Play", "Lecture", "Lire", "Wiedergabe", "Reproducir", "Riproduci", "Reproduzir", "Afspelen"];
const PAUSE: &[&str] = &["Pause", "Pausa", "Pauzeren", "Pausieren"];
const NEXT: &[&str] = &["Next", "Suivant", "Weiter", "Siguiente", "Successivo", "Avanti", "Próxima", "Volgende"];
const PREVIOUS: &[&str] = &["Previous", "Précédent", "Zurück", "Anterior", "Precedente", "Indietro", "Vorige"];
const SHUFFLE: &[&str] = &["shuffle", "aléatoire", "zufall", "aleatori", "willekeurig"];
const REPEAT: &[&str] = &["repeat", "répétition", "répéter", "wiederhol", "repetir", "ripeti", "herhaal"];
const LIKE: &[&str] = &[
    "liked songs",
    "titres likés",
    "lieblingssongs",
    "canciones que te gustan",
    "brani che ti piacciono",
    "músicas curtidas",
];

fn transport(names: &[&str]) -> Locator {
    Locator::role(Role::Button).any_of(names).lowest()
}

/// The play / pause button of the player bar. It is one button whose name says what it will do.
pub fn play_button() -> Locator {
    transport(PLAY)
}

pub fn pause_button() -> Locator {
    transport(PAUSE)
}

fn chord(mods: &[Modifier], key: Key) -> Option<KeyChord> {
    #![allow(clippy::unnecessary_wraps)] // every control has an optional shortcut; these all have one
    Some(KeyChord::new(mods, key))
}

/// Button and shortcut for a player action; `None` for actions that are not one button.
pub fn control(action: &SpotifyAction) -> Option<Control> {
    let c = |button, shortcut| Some(Control { button, shortcut });
    match action {
        SpotifyAction::Play => c(play_button(), None),
        SpotifyAction::Pause => c(pause_button(), None),
        SpotifyAction::Toggle => c(transport(PLAY).any_of(PAUSE), chord(&[], Key::Space)),
        SpotifyAction::Next => c(transport(NEXT), chord(&[Modifier::Ctrl], Key::Right)),
        SpotifyAction::Previous => c(transport(PREVIOUS), chord(&[Modifier::Ctrl], Key::Left)),
        SpotifyAction::Shuffle => c(transport(SHUFFLE).contains(), chord(&[Modifier::Ctrl], Key::Char('s'))),
        SpotifyAction::Repeat => c(transport(REPEAT).contains(), chord(&[Modifier::Ctrl], Key::Char('r'))),
        SpotifyAction::Like => c(transport(LIKE).contains(), chord(&[Modifier::Alt, Modifier::Shift], Key::Char('b'))),
        SpotifyAction::PlayQuery(_) | SpotifyAction::NowPlaying | SpotifyAction::Open => None,
    }
}

/// Play buttons on search results are named after what they play: "Play Get Lucky by Daft Punk",
/// "Lire Get Lucky par Daft Punk".
pub fn result_buttons() -> Locator {
    let prefixes: Vec<String> = PLAY.iter().map(|p| format!("{p} ")).collect();
    let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    Locator::role(Role::Button).any_of(&refs).prefix().except(PLAY).except(PAUSE)
}

/// Among the result play buttons, the one that best fits the query: the first whose name holds
/// every word of it, else the first result (Spotify ranks the top result first).
pub fn best_result(names: &[&str], query: &str) -> Option<usize> {
    let wanted: Vec<String> = normalize(query).split_whitespace().map(str::to_string).collect();
    names
        .iter()
        .position(|n| {
            let n = normalize(n);
            wanted.iter().all(|w| n.contains(w.as_str()))
        })
        .or_else(|| (!names.is_empty()).then_some(0))
}

/// "Play Get Lucky by Daft Punk" -> "Get Lucky by Daft Punk".
pub fn result_title(button: &str) -> String {
    let lower = normalize(button);
    PLAY.iter()
        .map(|p| normalize(p))
        .find(|p| lower.starts_with(&format!("{p} ")))
        .map_or_else(|| button.to_string(), |p| button.chars().skip(p.chars().count() + 1).collect())
}

pub fn is_spotify(w: &WindowInfo) -> bool {
    w.app() == "spotify"
}

/// The desktop app titles its window "Artist - Song" while something plays, and "Spotify",
/// "Spotify Premium" or "Spotify Free" when paused.
pub fn now_playing(title: &str) -> Option<String> {
    let t = title.trim();
    let n = normalize(t);
    if t.is_empty() || n == "spotify" || n.starts_with("spotify premium") || n.starts_with("spotify free") {
        return None;
    }
    Some(t.to_string())
}

/// A `spotify:` link that opens the desktop app on a search.
pub fn search_uri(query: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::ui::Element;

    #[test]
    fn transport_buttons_in_french_and_english() {
        let bar = |n: &str| Element::new(Role::Button, n).at(Rect::new(0.0, 1000.0, 32.0, 32.0));
        let els = [
            Element::new(Role::Button, "Lecture").at(Rect::new(0.0, 300.0, 56.0, 56.0)),
            bar("Précédent"),
            bar("Lecture"),
            bar("Suivant"),
        ];
        assert_eq!(control(&SpotifyAction::Next).unwrap().button.pick(&els), Some(3));
        assert_eq!(control(&SpotifyAction::Previous).unwrap().button.pick(&els), Some(1));
        assert_eq!(play_button().pick(&els), Some(2), "the player bar, not the playlist's big button");
        assert_eq!(pause_button().pick(&els), None, "already paused");
    }

    #[test]
    fn toggles_match_whatever_state_they_are_in() {
        let els = [
            Element::new(Role::Button, "Désactiver la lecture aléatoire"),
            Element::new(Role::Button, "Enable repeat one"),
        ];
        assert_eq!(control(&SpotifyAction::Shuffle).unwrap().button.pick(&els), Some(0));
        assert_eq!(control(&SpotifyAction::Repeat).unwrap().button.pick(&els), Some(1));
        let like = [Element::new(Role::Button, "Ajouter aux Titres likés")];
        assert_eq!(control(&SpotifyAction::Like).unwrap().button.pick(&like), Some(0));
    }

    #[test]
    fn results_skip_the_transport_and_fit_the_query() {
        let els = [
            Element::new(Role::Button, "Play"),
            Element::new(Role::Button, "Play Daft Punk"),
            Element::new(Role::Button, "Play Get Lucky by Daft Punk"),
        ];
        let hits: Vec<&str> = els.iter().filter(|e| result_buttons().matches(e)).map(|e| e.name.as_str()).collect();
        assert_eq!(hits, ["Play Daft Punk", "Play Get Lucky by Daft Punk"]);
        assert_eq!(best_result(&hits, "get lucky"), Some(1));
        assert_eq!(best_result(&hits, "daft punk"), Some(0));
        assert_eq!(best_result(&hits, "something else"), Some(0));
        assert_eq!(result_title("Lire Get Lucky par Daft Punk"), "Get Lucky par Daft Punk");
    }

    #[test]
    fn the_title_says_what_plays() {
        assert_eq!(now_playing("Daft Punk - Get Lucky").as_deref(), Some("Daft Punk - Get Lucky"));
        assert_eq!(now_playing("Spotify Premium"), None);
        assert_eq!(now_playing("Spotify"), None);
    }

    #[test]
    fn search_links_are_encoded() {
        assert_eq!(search_uri("daft punk"), "spotify:search:daft%20punk");
        assert_eq!(search_uri("Beyoncé"), "spotify:search:Beyonc%C3%A9");
    }
}
