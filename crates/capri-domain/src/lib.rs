//! # capri-domain
//!
//! The innermost ring of the hexagon. Everything here is plain data and pure logic: no files, no
//! network, no windows, no clock. Time is passed in as seconds, randomness comes from a seeded [`Rng`],
//! and the desktop arrives as a list of [`Ledge`]s. That keeps all of Capri's behaviour unit-testable.

pub mod animator;
pub mod approval;
pub mod character;
pub mod clip;
pub mod conversation;
pub mod geometry;
pub mod intent;
pub mod language;
pub mod ledge;
pub mod rng;
pub mod sentences;

pub use animator::{Animator, Frame};
pub use approval::{ApprovalMode, Decision, Origin, Risk};
pub use character::{Character, CharacterConfig, Happening, Mode, Pose};
pub use clip::{ClipId, ClipSpec, Playback};
pub use conversation::{Conversation, Message, Persona, Role};
pub use geometry::{Point, Rect};
pub use intent::Intent;
pub use language::Language;
pub use ledge::{Ledge, LedgeId, LedgeKind};
pub use rng::Rng;
pub use sentences::SentenceSplitter;
