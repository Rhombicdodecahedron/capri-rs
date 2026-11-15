//! Music commands understood without the model: instant, free, and the same for every brain.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "query", rename_all = "snake_case")]
pub enum MediaCommand {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    Mute,
    OpenSpotify,
    /// Look something up in Spotify.
    Search(String),
}

impl MediaCommand {
    /// Her short answer once it is done.
    pub fn reply(&self, lang: crate::Language) -> String {
        let t = |en: &str, fr: &str| lang.pick(en, fr).to_string();
        match self {
            MediaCommand::PlayPause => t("Done.", "C'est fait."),
            MediaCommand::Play => t("Music's back on.", "C'est reparti."),
            MediaCommand::Pause => t("Paused.", "En pause."),
            MediaCommand::Next => t("Next one.", "Suivante."),
            MediaCommand::Previous => t("Going back.", "Je reviens en arrière."),
            MediaCommand::VolumeUp => t("Louder.", "Plus fort."),
            MediaCommand::VolumeDown => t("Quieter.", "Moins fort."),
            MediaCommand::Mute => t("Sound off.", "Son coupé."),
            MediaCommand::OpenSpotify => t("Opening Spotify.", "J'ouvre Spotify."),
            MediaCommand::Search(q) => {
                format!("{} {q}", lang.pick("Looking up on Spotify:", "Je cherche sur Spotify :"))
            }
        }
    }
}

/// Words that only make the request polite, dropped before matching.
const FILLER: &[&str] =
    &["capri", "stp", "svp", "please", "pls", "steuplait", "merci", "thanks", "thx", "hey", "hé", "eh", "ok", "okay"];

/// Lowercase words without punctuation or filler.
fn words(line: &str) -> Vec<String> {
    let lower = line.to_lowercase().replace(['’', '\''], "' ");
    let mut out: Vec<String> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ';' | ':'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    // "s'il te plaît" / "s'il vous plaît"
    let joined = out.join(" ");
    for phrase in ["s' il te plaît", "s' il te plait", "s' il vous plaît", "s' il vous plait"] {
        if joined.contains(phrase) {
            let cleaned = joined.replace(phrase, " ");
            out = cleaned.split_whitespace().map(str::to_string).collect();
        }
    }
    out.retain(|w| !FILLER.contains(&w.as_str()));
    out
}

/// Whole-line commands, in French and English.
const EXACT: &[(MediaCommand, &[&str])] = &[
    (
        MediaCommand::Pause,
        &[
            "pause",
            "mets pause",
            "mets en pause",
            "mets la musique en pause",
            "pause la musique",
            "stop la musique",
            "arrête la musique",
            "arrete la musique",
            "coupe la musique",
            "pause the music",
            "stop the music",
            "pause music",
        ],
    ),
    (
        MediaCommand::Play,
        &[
            "play",
            "resume",
            "reprends",
            "reprend",
            "relance",
            "relance la musique",
            "remets la musique",
            "mets la musique",
            "lance la musique",
            "reprends la musique",
            "play music",
            "play the music",
            "resume the music",
            "lecture",
        ],
    ),
    (MediaCommand::PlayPause, &["play pause", "play/pause", "lecture pause"]),
    (
        MediaCommand::Next,
        &[
            "suivant",
            "suivante",
            "next",
            "skip",
            "passe",
            "zappe",
            "chanson suivante",
            "musique suivante",
            "morceau suivant",
            "titre suivant",
            "next song",
            "next track",
            "skip this",
            "skip this song",
            "passe à la suivante",
            "passe a la suivante",
            "change de musique",
        ],
    ),
    (
        MediaCommand::Previous,
        &[
            "précédent",
            "précédente",
            "precedent",
            "precedente",
            "previous",
            "chanson précédente",
            "musique précédente",
            "morceau précédent",
            "titre précédent",
            "previous song",
            "previous track",
            "go back",
            "reviens en arrière",
        ],
    ),
    (
        MediaCommand::VolumeUp,
        &[
            "monte le son",
            "monte le volume",
            "plus fort",
            "augmente le son",
            "augmente le volume",
            "volume up",
            "louder",
            "turn it up",
        ],
    ),
    (
        MediaCommand::VolumeDown,
        &[
            "baisse le son",
            "baisse le volume",
            "moins fort",
            "diminue le son",
            "diminue le volume",
            "volume down",
            "quieter",
            "turn it down",
        ],
    ),
    (MediaCommand::Mute, &["mute", "coupe le son", "silence", "unmute", "remets le son"]),
    (
        MediaCommand::OpenSpotify,
        &["lance spotify", "ouvre spotify", "open spotify", "launch spotify", "start spotify", "démarre spotify"],
    ),
];

/// "mets X sur spotify", "play X on spotify"...
const SEARCH: &[(&[&str], &[&str])] = &[
    (&["mets", "joue", "lance", "cherche", "trouve"], &[" sur spotify", " dans spotify"]),
    (&["play", "put on", "search", "find"], &[" on spotify", " in spotify"]),
];

/// The music command in `line`, if the whole line is one.
pub fn parse(line: &str) -> Option<MediaCommand> {
    let w = words(line);
    if w.is_empty() || w.len() > 9 {
        return None;
    }
    let line = w.join(" ");
    if let Some((cmd, _)) = EXACT.iter().find(|(_, phrases)| phrases.contains(&line.as_str())) {
        return Some(cmd.clone());
    }
    for (verbs, tails) in SEARCH {
        for v in *verbs {
            for tail in *tails {
                let found = line.strip_prefix(&format!("{v} ")).and_then(|r| r.strip_suffix(tail)).map(str::trim);
                if let Some(q) = found.filter(|q| !q.is_empty()) {
                    return Some(MediaCommand::Search(q.to_string()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn french_commands() {
        assert_eq!(parse("Capri, mets en pause stp"), Some(MediaCommand::Pause));
        assert_eq!(parse("chanson suivante !"), Some(MediaCommand::Next));
        assert_eq!(parse("Monte le son s'il te plaît"), Some(MediaCommand::VolumeUp));
        assert_eq!(parse("Lance Spotify"), Some(MediaCommand::OpenSpotify));
        assert_eq!(parse("mets Daft Punk sur Spotify"), Some(MediaCommand::Search("daft punk".into())));
    }

    #[test]
    fn english_commands() {
        assert_eq!(parse("Next track please"), Some(MediaCommand::Next));
        assert_eq!(parse("play Resonance on Spotify"), Some(MediaCommand::Search("resonance".into())));
        assert_eq!(parse("pause"), Some(MediaCommand::Pause));
    }

    #[test]
    fn conversation_is_not_a_command() {
        assert_eq!(parse("j'aime bien cette chanson, elle est de qui ?"), None);
        assert_eq!(parse("est-ce que tu peux mettre un peu de musique chill pendant que je code ce soir"), None);
        assert_eq!(parse("pause café ?"), None);
    }
}
