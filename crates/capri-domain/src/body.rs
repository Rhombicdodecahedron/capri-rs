//! Things you can ask her body to do, typed or said: take a nap, go mini, lie down, wake up. Whole
//! lines only, like the music commands, so a sentence that merely mentions sleep stays a chat.

use serde::{Deserialize, Serialize};

use crate::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyCommand {
    /// A nap in her mini form on the taskbar: eyes shut, z's; she wakes up full size after a while.
    Nap,
    /// Small and awake, until asked back.
    Mini,
    /// Back to full size (out of mini or a nap).
    FullSize,
    /// Lie down where she is, on the taskbar or on top of a window.
    LieDown,
    /// Out of any sleep or lying down.
    WakeUp,
}

const PHRASES: &[(BodyCommand, &[&str])] = &[
    (
        BodyCommand::Nap,
        &[
            "va dormir",
            "va faire dodo",
            "fais dodo",
            "dors",
            "fais une sieste",
            "va faire une sieste",
            "fais une pause",
            "fais un somme",
            "repose-toi",
            "va te reposer",
            "go to sleep",
            "go take a nap",
            "take a nap",
            "nap",
            "take a break",
            "go chill",
            "sleep",
        ],
    ),
    (
        BodyCommand::Mini,
        &[
            "passe en mini",
            "mode mini",
            "en mini",
            "fais-toi petite",
            "rapetisse",
            "go mini",
            "mini mode",
            "be small",
            "get small",
        ],
    ),
    (
        BodyCommand::FullSize,
        &[
            "reviens",
            "reviens en grand",
            "grande taille",
            "en grand",
            "taille normale",
            "sors du mini",
            "come back",
            "full size",
            "big again",
            "go big",
        ],
    ),
    (
        BodyCommand::LieDown,
        &["allonge-toi", "couche-toi", "va t' allonger", "va te coucher", "lie down", "lay down", "go lie down"],
    ),
    (
        BodyCommand::WakeUp,
        &["reveille-toi", "debout", "leve-toi", "wake up", "get up", "rise and shine", "reveille toi", "leve toi"],
    ),
];

pub fn parse(line: &str) -> Option<BodyCommand> {
    let line = crate::words::request(line).join(" ");
    if line.is_empty() {
        return None;
    }
    let same = |p: &&str| crate::words::request(p).join(" ") == line;
    PHRASES.iter().find(|(_, phrases)| phrases.iter().any(same)).map(|(c, _)| *c)
}

impl BodyCommand {
    /// Her short answer as she does it.
    pub fn reply(self, lang: Language) -> &'static str {
        match self {
            BodyCommand::Nap => {
                lang.pick("Five minutes. Wake me if you need me.", "Cinq minutes. Réveille-moi si t'as besoin.")
            }
            BodyCommand::Mini => lang.pick("Going small.", "Je me fais petite."),
            BodyCommand::FullSize => lang.pick("Back.", "Me revoilà."),
            BodyCommand::LieDown => lang.pick("Lying down for a bit.", "Je m'allonge un peu."),
            BodyCommand::WakeUp => lang.pick("I'm up, I'm up.", "C'est bon, je suis debout."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_lines_in_french_and_english() {
        assert_eq!(parse("Capri, va faire une sieste stp"), Some(BodyCommand::Nap));
        assert_eq!(parse("passe en mini !"), Some(BodyCommand::Mini));
        assert_eq!(parse("Allonge-toi"), Some(BodyCommand::LieDown));
        assert_eq!(parse("réveille-toi"), Some(BodyCommand::WakeUp));
        assert_eq!(parse("come back"), Some(BodyCommand::FullSize));
        assert_eq!(parse("go to sleep"), Some(BodyCommand::Nap));
    }

    #[test]
    fn mentions_stay_conversation() {
        assert_eq!(parse("j'ai mal dormi cette nuit"), None);
        assert_eq!(parse("tu dors quand toi ?"), None);
        assert_eq!(parse(""), None);
    }
}
