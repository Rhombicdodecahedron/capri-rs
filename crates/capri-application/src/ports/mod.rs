//! Ports: the traits the application needs from the outside world. Adapters implement them; the
//! composition root (`apps/capri`) plugs adapters into services. Nothing in here knows about HTTP,
//! Win32, SQLite or pixels on a screen.

pub mod desktop;
pub mod llm;
pub mod memory;
pub mod skin;
pub mod speech;
pub mod tools;
pub mod ui;

pub use desktop::{DesktopProbe, Tracking};
pub use llm::{LanguageModel, LlmError, Piece, PieceStream, TokenStream};
pub use memory::{MemoryError, MemoryStore};
pub use skin::{Skin, SkinError, SkinInfo, SkinRepository, Sprite, Strip};
pub use speech::{Audio, Microphone, Speaker, SpeechError, SpeechToText, TextToSpeech};
pub use tools::{ActionRequest, ApprovalPrompt, Tool, ToolError, ToolSpec};
pub use ui::{Opener, UiAction, UiDriver, UiError};
