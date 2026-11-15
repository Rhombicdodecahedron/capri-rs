use std::collections::HashMap;

use capri_domain::ClipId;

/// One decoded frame: premultiplied RGBA8, row-major, `width * height * 4` bytes.
#[derive(Clone)]
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Sprite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sprite({}x{})", self.width, self.height)
    }
}

impl Sprite {
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
        self.pixels[((y * self.width + x) * 4 + 3) as usize]
    }
}

/// A clip cut into frames and scaled to her on-screen size.
#[derive(Debug, Clone)]
pub struct Strip {
    pub clip: ClipId,
    pub frames: Vec<Sprite>,
    /// The point of the frame that sits on her anchor (feet or seat), as fractions of the frame size.
    pub anchor: (f32, f32),
}

#[derive(Debug, Clone)]
pub struct SkinInfo {
    pub id: String,
    pub name: String,
    pub credits: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Skin {
    pub info: SkinInfo,
    pub strips: HashMap<ClipId, Strip>,
}

impl Skin {
    pub fn strip(&self, clip: ClipId) -> Option<&Strip> {
        self.strips.get(&clip)
    }

    /// The strip for `clip`, or the closest one this skin has: a skin only needs `idle` to work,
    /// a missing `perch_blink` falls back to `perch`, then to `idle`.
    pub fn strip_or_fallback(&self, clip: ClipId) -> Option<&Strip> {
        let mut c = Some(clip);
        while let Some(id) = c {
            if let Some(s) = self.strips.get(&id) {
                return Some(s);
            }
            c = id.fallback();
        }
        None
    }

    /// Whether this skin can lie down (it draws `lay_awake`).
    pub fn can_lie_down(&self) -> bool {
        self.strips.contains_key(&ClipId::LayAwake)
    }

    /// Whether this skin has a mini form (it draws `mini/idle`).
    pub fn has_mini(&self) -> bool {
        self.strips.contains_key(&ClipId::MiniIdle)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkinError {
    #[error("skin `{0}` not found")]
    NotFound(String),
    #[error("skin `{skin}`: {file}: {reason}")]
    Invalid { skin: String, file: String, reason: String },
}

/// Driven port: where her looks come from. The filesystem adapter reads the same sprite-strip format
/// as the original project, so a skin folder can be dropped in unchanged.
pub trait SkinRepository: Send + Sync {
    fn list(&self) -> Vec<SkinInfo>;

    /// Loads skin `id` with frames scaled for a character of `size` screen px at `scale` (DPI factor).
    fn load(&self, id: &str, size: f32, scale: f32) -> Result<Skin, SkinError>;
}
