//! Which actions need the user's OK. The policy lives in the domain so no adapter can skip it:
//! the application layer asks this before running any tool.

use serde::{Deserialize, Serialize};

/// How much an action can change on the user's machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Reads only: weather, the time, listing a folder.
    Read,
    /// Changes something easy to undo: opening an app, playing music.
    Reversible,
    /// Hard to undo or reaches outside: deleting or moving files, running shell commands, sending messages.
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Ask before anything that changes the machine.
    #[default]
    Ask,
    /// Run reversible actions directly, still ask for destructive ones.
    Auto,
    /// Never change anything.
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AskUser,
    Deny,
}

/// Who asked for the action. Only the user's own input can approve; text from the model, a web page
/// or someone on a call can request but never approve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    User,
    Model,
}

pub fn decide(mode: ApprovalMode, risk: Risk) -> Decision {
    match (mode, risk) {
        (_, Risk::Read) | (ApprovalMode::Auto, Risk::Reversible) => Decision::Allow,
        (ApprovalMode::ReadOnly, _) => Decision::Deny,
        (ApprovalMode::Ask | ApprovalMode::Auto, _) => Decision::AskUser,
    }
}

/// Like [`decide`], knowing who asked: a reversible action the user asked for directly runs without a
/// second prompt, while destructive actions are always confirmed, whoever asked.
pub fn decide_for(mode: ApprovalMode, risk: Risk, origin: Origin) -> Decision {
    let d = decide(mode, risk);
    if d == Decision::AskUser && origin == Origin::User && risk == Risk::Reversible {
        return Decision::Allow;
    }
    d
}

/// The user's answer to "can I?": `Some(true)` for yes, `Some(false)` for no, `None` when the line
/// is something else (they moved on, which counts as no).
pub fn answer(line: &str) -> Option<bool> {
    const YES: &[&str] = &[
        "oui",
        "ouais",
        "ouai",
        "ouep",
        "yes",
        "yep",
        "yeah",
        "y",
        "o",
        "ok",
        "okay",
        "vas-y",
        "vas y",
        "go",
        "d' accord",
        "dac",
        "bien sur",
        "carrement",
        "fais-le",
        "fais le",
        "fais",
        "do it",
        "sure",
        "allez",
        "allez-y",
        "volontiers",
        "oui vas-y",
        "oui stp",
        "yes please",
        "ja",
        "si",
        "confirm",
        "confirme",
        "valide",
    ];
    const NO: &[&str] = &[
        "non",
        "nan",
        "no",
        "nope",
        "n",
        "annule",
        "cancel",
        "stop",
        "laisse",
        "laisse tomber",
        "pas maintenant",
        "surtout pas",
        "never",
        "jamais",
        "nein",
        "arrete",
        "non merci",
        "no thanks",
    ];
    let w = crate::words::words(line);
    let w: Vec<&str> = w.iter().map(String::as_str).filter(|w| !matches!(*w, "capri" | "merci" | "thanks")).collect();
    let line = w.join(" ");
    let first = w.first().copied().unwrap_or_default();
    if NO.contains(&line.as_str()) || (w.len() <= 4 && matches!(first, "non" | "no" | "nan" | "nope")) {
        return Some(false);
    }
    if YES.contains(&line.as_str())
        || (w.len() <= 4 && matches!(first, "oui" | "ouais" | "yes" | "ok" | "vas-y" | "go"))
    {
        return Some(true);
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn yes_and_no_in_french_and_english() {
        for y in ["Oui", "ouais vas-y", "OK!", "yes please", "d'accord", "vas-y Capri"] {
            assert_eq!(answer(y), Some(true), "{y}");
        }
        for n in ["non", "Non merci", "nope", "annule", "laisse tomber"] {
            assert_eq!(answer(n), Some(false), "{n}");
        }
        assert_eq!(answer("attends, mets plutôt du jazz"), None);
    }

    use super::*;

    #[test]
    fn reads_are_always_allowed() {
        for m in [ApprovalMode::Ask, ApprovalMode::Auto, ApprovalMode::ReadOnly] {
            assert_eq!(decide(m, Risk::Read), Decision::Allow);
        }
    }

    #[test]
    fn destructive_always_asks_or_denies() {
        assert_eq!(decide(ApprovalMode::Auto, Risk::Destructive), Decision::AskUser);
        assert_eq!(decide(ApprovalMode::ReadOnly, Risk::Destructive), Decision::Deny);
        assert_eq!(decide_for(ApprovalMode::Ask, Risk::Destructive, Origin::User), Decision::AskUser);
    }

    #[test]
    fn model_cannot_self_approve() {
        assert_eq!(decide_for(ApprovalMode::Ask, Risk::Reversible, Origin::Model), Decision::AskUser);
        assert_eq!(decide_for(ApprovalMode::Ask, Risk::Reversible, Origin::User), Decision::Allow);
    }
}
