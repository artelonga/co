use std::fmt;

/// Minimal error type — no #[track_caller], no source path embedding.
#[derive(Debug)]
pub enum GameError {
    Io(String),
    Parse(String),
    Script(String),
    Terminal(String),
    Command(String),
    InvalidState(String),
    Storage(String),
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::Io(msg) => write!(f, "{}", msg),
            GameError::Parse(msg) => write!(f, "{}", msg),
            GameError::Script(msg) => write!(f, "{}", msg),
            GameError::Terminal(msg) => write!(f, "{}", msg),
            GameError::Command(msg) => write!(f, "{}", msg),
            GameError::InvalidState(msg) => write!(f, "{}", msg),
            GameError::Storage(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for GameError {}

impl From<std::io::Error> for GameError {
    fn from(e: std::io::Error) -> Self {
        GameError::Io(e.to_string())
    }
}

impl From<std::num::ParseIntError> for GameError {
    fn from(_: std::num::ParseIntError) -> Self {
        GameError::Parse(String::new())
    }
}

pub type Result<T> = std::result::Result<T, GameError>;

/// Safe indexing — aborts on out-of-bounds instead of panicking.
/// Unlike `[]`, this does not embed source file paths via `#[track_caller]`.
#[inline(always)]
pub fn at<T>(slice: &[T], index: usize) -> &T {
    match slice.get(index) {
        Some(v) => v,
        None => std::process::abort(),
    }
}

/// Mutable safe indexing — aborts on out-of-bounds instead of panicking.
#[inline(always)]
pub fn at_mut<T>(slice: &mut [T], index: usize) -> &mut T {
    match slice.get_mut(index) {
        Some(v) => v,
        None => std::process::abort(),
    }
}
