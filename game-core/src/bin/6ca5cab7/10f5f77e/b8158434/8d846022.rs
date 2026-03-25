/// Cinema data model and constructor DSL.
///
/// Types are consumed by `content.rs` (producer) and `player.rs` (consumer).
/// Constructor functions provide a clean syntax for defining movie content.
///
/// A complete movie — a sequence of scenes played in order.
pub struct Movie {
    pub scenes: Vec<Scene>,
}

/// A scene — a sequence of acts played on a cleared screen.
pub struct Scene {
    pub acts: Vec<Act>,
}

/// An individual act within a scene.
pub enum Act {
    /// Typewriter text at an explicit (row, col) position.
    Type {
        row: u16,
        col: u16,
        text: String,
        ms_per_word: u64,
        style: Style,
    },
    /// Typewriter text centered horizontally on the given row.
    Center {
        row: u16,
        text: String,
        ms_per_word: u64,
        style: Style,
    },
    /// Instant print at explicit position (no typewriter delay).
    Print {
        row: u16,
        col: u16,
        text: String,
        style: Style,
    },
    /// Pause for the given duration in milliseconds.
    Wait(u64),
    /// Fade transition: pause → clear → pause.
    Fade(u64),
}

/// Text styling attributes.
#[derive(Clone, Default)]
pub struct Style {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub color: Option<Color>,
}

/// Color palette for cinema text.
#[derive(Clone, Copy)]
pub enum Color {
    Yellow,
    Cyan,
    Green,
    Red,
    Magenta,
    White,
    Grey,
}

// ── Movie constructor ──────────────────────────────────────────

impl Movie {
    pub fn from(scenes: Vec<Scene>) -> Self {
        Self { scenes }
    }
}

// ── Scene constructor ──────────────────────────────────────────

pub fn scene(acts: Vec<Act>) -> Scene {
    Scene { acts }
}

// ── Act constructors ───────────────────────────────────────────

/// Typewriter text at explicit position.
pub fn typew(row: u16, col: u16, text: &str, ms_per_word: u64, style: Style) -> Act {
    Act::Type {
        row,
        col,
        text: text.to_string(),
        ms_per_word,
        style,
    }
}

/// Typewriter text centered on the given row.
pub fn center(row: u16, text: &str, ms_per_word: u64, style: Style) -> Act {
    Act::Center {
        row,
        text: text.to_string(),
        ms_per_word,
        style,
    }
}

/// Instant print at explicit position.
pub fn print_at(row: u16, col: u16, text: &str, style: Style) -> Act {
    Act::Print {
        row,
        col,
        text: text.to_string(),
        style,
    }
}

/// Pause for given milliseconds.
pub fn wait(ms: u64) -> Act {
    Act::Wait(ms)
}

/// Fade transition (pause → clear → pause).
pub fn fade(ms: u64) -> Act {
    Act::Fade(ms)
}

// ── Style constructors ─────────────────────────────────────────

/// Plain unstyled text with optional color.
pub fn plain(color: Option<Color>) -> Style {
    Style {
        color,
        ..Style::default()
    }
}

/// Bold text with optional color.
pub fn bold(color: Option<Color>) -> Style {
    Style {
        bold: true,
        color,
        ..Style::default()
    }
}

/// Dim text with optional color.
pub fn dim(color: Option<Color>) -> Style {
    Style {
        dim: true,
        color,
        ..Style::default()
    }
}

/// Italic text with optional color.
pub fn italic(color: Option<Color>) -> Style {
    Style {
        italic: true,
        color,
        ..Style::default()
    }
}

// ── Color constructors (return Option<Color> for style ergonomics) ──

pub fn yellow() -> Option<Color> {
    Some(Color::Yellow)
}

pub fn cyan() -> Option<Color> {
    Some(Color::Cyan)
}

pub fn green() -> Option<Color> {
    Some(Color::Green)
}

pub fn red() -> Option<Color> {
    Some(Color::Red)
}

pub fn magenta() -> Option<Color> {
    Some(Color::Magenta)
}

pub fn white() -> Option<Color> {
    Some(Color::White)
}

pub fn grey() -> Option<Color> {
    Some(Color::Grey)
}
