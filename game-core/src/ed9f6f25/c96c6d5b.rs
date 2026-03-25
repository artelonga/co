use crossterm::event::{self, Event, KeyCode, KeyModifiers};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Move(Direction),
    Action,
    Quit,
    None,
    // Poker-specific inputs
    Fold,
    Check,
    Call,
    Raise,
    AllIn,
    // Text input
    Char(char),
    Backspace,
    // Number input for betting
    Number(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Standard input polling (movement, action, quit)
pub fn poll_input() -> super::Result<Input> {
    if event::poll(std::time::Duration::from_millis(16))? {
        if let Event::Key(key) = event::read()? {
            return Ok(match key.code {
                KeyCode::Up => Input::Move(Direction::Up),
                KeyCode::Down => Input::Move(Direction::Down),
                KeyCode::Left => Input::Move(Direction::Left),
                KeyCode::Right => Input::Move(Direction::Right),
                KeyCode::Enter => Input::Action,
                KeyCode::Char('q') | KeyCode::Char('Q') => Input::Quit,
                KeyCode::Esc => Input::Quit,
                _ => Input::None,
            });
        }
    }
    Ok(Input::None)
}

/// Extended input polling for poker (includes poker actions and text input)
pub fn poll_poker_input() -> super::Result<Input> {
    if event::poll(std::time::Duration::from_millis(16))? {
        if let Event::Key(key) = event::read()? {
            // Check for Ctrl+C to quit
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                if let KeyCode::Char('c') = key.code {
                    return Ok(Input::Quit);
                }
            }

            return Ok(match key.code {
                // Navigation
                KeyCode::Up => Input::Move(Direction::Up),
                KeyCode::Down => Input::Move(Direction::Down),
                KeyCode::Left => Input::Move(Direction::Left),
                KeyCode::Right => Input::Move(Direction::Right),
                KeyCode::Enter => Input::Action,
                KeyCode::Esc => Input::Quit,

                // Poker actions (case-insensitive)
                KeyCode::Char('f') | KeyCode::Char('F') => Input::Fold,
                KeyCode::Char('c') | KeyCode::Char('C') => Input::Check, // Also used for Call contextually
                KeyCode::Char('k') | KeyCode::Char('K') => Input::Call,
                KeyCode::Char('r') | KeyCode::Char('R') => Input::Raise,
                KeyCode::Char('a') | KeyCode::Char('A') => Input::AllIn,

                // Number keys for betting amounts
                KeyCode::Char(c @ '0'..='9') => Input::Number((c as u8) - b'0'),

                // Text input for chat
                KeyCode::Char(c) => Input::Char(c),
                KeyCode::Backspace => Input::Backspace,

                _ => Input::None,
            });
        }
    }
    Ok(Input::None)
}

/// Input mode for different game states
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    /// Navigation mode (arrows + action)
    Navigation,
    /// Poker action selection
    PokerAction,
    /// Text input mode (chat)
    TextInput,
    /// Numeric input (betting amount)
    NumericInput,
}

/// Text input buffer for chat messages
#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    pub content: String,
    pub cursor: usize,
    pub max_length: usize,
}

impl TextBuffer {
    pub fn new(max_length: usize) -> Self {
        Self {
            content: String::new(),
            cursor: 0,
            max_length,
        }
    }

    pub fn insert(&mut self, c: char) {
        if self.content.len() < self.max_length {
            self.content.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.content.remove(self.cursor);
        }
    }

    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor = 0;
    }

    pub fn take(&mut self) -> String {
        let content = std::mem::take(&mut self.content);
        self.cursor = 0;
        content
    }
}
