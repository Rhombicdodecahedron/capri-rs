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
    /// Find it on Spotify and start it.
    Search(String),
    Shuffle,
    Repeat,
    /// Add the current song to Liked Songs.
    Like,
    /// What song is this?
    NowPlaying,
}

impl MediaCommand {
    /// Her short answer once it is done. `result` is what the tool reported (the song found or
    /// playing), when it said something useful.
    pub fn reply(&self, lang: crate::Language, result: &str) -> String {
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
            MediaCommand::Search(q) => match result.strip_prefix("playing ") {
                Some(song) => format!("{} {song}.", lang.pick("Playing", "Je lance")),
                None => format!("{} {q}", lang.pick("Looking up on Spotify:", "Je cherche sur Spotify :")),
            },
            MediaCommand::Shuffle => t("Shuffle switched.", "Aléatoire basculé."),
            MediaCommand::Repeat => t("Repeat switched.", "Répétition basculée."),
            MediaCommand::Like => t("Added to your Liked Songs.", "Ajoutée à tes titres likés."),
            MediaCommand::NowPlaying => match result.strip_prefix("playing ") {
                Some(song) => format!("{} {song}.", lang.pick("That's", "C'est")),
                None => t("Nothing is playing right now.", "Rien ne joue en ce moment."),
            },
        }
    }
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
        MediaCommand::Shuffle,
        &[
            "aleatoire",
            "mode aleatoire",
            "mets en aleatoire",
            "lecture aleatoire",
            "active l' aleatoire",
            "shuffle",
            "shuffle on",
            "turn on shuffle",
        ],
    ),
    (
        MediaCommand::Repeat,
        &["repete", "en boucle", "mets en boucle", "repetition", "repeat", "repeat this", "loop", "on repeat"],
    ),
    (
        MediaCommand::Like,
        &[
            "like",
            "like la",
            "like le",
            "like ce son",
            "like cette chanson",
            "like cette musique",
            "like ce titre",
            "ajoute aux titres likes",
            "ajoute la aux titres likes",
            "like this",
            "like this song",
            "like it",
            "add to liked songs",
            "save this song",
        ],
    ),
    (
        MediaCommand::NowPlaying,
        &[
            "c' est quoi cette musique",
            "c' est quoi cette chanson",
            "c' est quoi ce son",
            "c' est quoi ce titre",
            "quelle est cette chanson",
            "quelle est cette musique",
            "qu' est-ce qui joue",
            "qu' est ce qui joue",
            "qu' est-ce que j' ecoute",
            "c' est qui",
            "what' s playing",
            "what is playing",
            "what song is this",
            "what' s this song",
            "who is this",
        ],
    ),
    (
        MediaCommand::OpenSpotify,
        &["lance spotify", "ouvre spotify", "open spotify", "launch spotify", "start spotify", "demarre spotify"],
    ),
];

/// "mets X sur spotify", "play X on spotify"...
const SEARCH: &[(&[&str], &[&str])] = &[
    (&["mets", "joue", "lance", "ecoute", "cherche", "trouve"], &[" sur spotify", " dans spotify"]),
    (&["play", "put on", "search", "find"], &[" on spotify", " in spotify"]),
];

/// The music command in `line`, if the whole line is one.
pub fn parse(line: &str) -> Option<MediaCommand> {
    let w = crate::words::request(line);
    if w.is_empty() || w.len() > 9 {
        return None;
    }
    let line = w.join(" ");
    // phrases go through the same normalisation, so they can be written naturally
    let same = |p: &&str| crate::words::request(p).join(" ") == line;
    if let Some((cmd, _)) = EXACT.iter().find(|(_, phrases)| phrases.iter().any(same)) {
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
    fn player_toggles_and_questions() {
        assert_eq!(parse("mets en aléatoire stp"), Some(MediaCommand::Shuffle));
        assert_eq!(parse("Like cette chanson"), Some(MediaCommand::Like));
        assert_eq!(parse("c'est quoi cette musique ?"), Some(MediaCommand::NowPlaying));
        assert_eq!(parse("What's playing?"), Some(MediaCommand::NowPlaying));
        assert_eq!(parse("écoute Stromae sur Spotify"), Some(MediaCommand::Search("stromae".into())));
        assert_eq!(parse("arrête la musique"), Some(MediaCommand::Pause));
        assert_eq!(parse("chanson précédente"), Some(MediaCommand::Previous));
    }

    #[test]
    fn conversation_is_not_a_command() {
        assert_eq!(parse("j'aime pas trop le like, tu trouves ?"), None);
        assert_eq!(parse("j'aime bien cette chanson, elle est de qui ?"), None);
        assert_eq!(parse("est-ce que tu peux mettre un peu de musique chill pendant que je code ce soir"), None);
        assert_eq!(parse("pause café ?"), None);
    }
}
