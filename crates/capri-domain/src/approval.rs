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

#[cfg(test)]
mod tests {
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
