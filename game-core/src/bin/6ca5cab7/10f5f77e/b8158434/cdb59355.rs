/// Cinema rendering engine — typewriter effects, transitions, input handling.
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Attribute, Color as CtColor, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType},
};
use obfstr::obfstr;
use std::io::{self, Write};
use std::time::Duration;

use super::types::{Act, Color, Movie, Style};
use game_core::engine::GameError;

/// Playback control signal from input polling.
enum Signal {
    None,
    Quit,
    Pause,
    SkipScene,
    Faster,
    Slower,
}

/// Result of executing an act.
enum Playback {
    Continue,
    SkipScene,
    Quit,
}

/// Cinema player — renders a Movie to the terminal.
pub struct CinemaPlayer {
    /// Speed multiplier. 1.0 = normal. Higher = slower delays.
    speed: f64,
    /// Terminal dimensions.
    term_width: u16,
    #[allow(dead_code)]
    term_height: u16,
    /// Row for the status line (bottom of screen).
    status_row: u16,
}

impl CinemaPlayer {
    pub fn new() -> Self {
        let (w, h) = terminal::size().unwrap_or((80, 24));
        Self {
            speed: 1.0,
            term_width: w,
            term_height: h,
            status_row: h.saturating_sub(1),
        }
    }

    /// Play an entire movie. Manages terminal setup/cleanup.
    pub fn play(&mut self, movie: &Movie) -> game_core::Result<()> {
        self.enter()?;
        let result = self.run(movie);
        self.leave()?;
        result
    }

    // ── Terminal management ────────────────────────────────────

    fn enter(&self) -> game_core::Result<()> {
        terminal::enable_raw_mode().map_err(|e| GameError::Io(e.to_string()))?;
        let mut out = io::stdout();
        execute!(out, terminal::EnterAlternateScreen, cursor::Hide)
            .map_err(|e| GameError::Io(e.to_string()))?;
        Ok(())
    }

    fn leave(&self) -> game_core::Result<()> {
        let mut out = io::stdout();
        let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        Ok(())
    }

    // ── Main playback loop ─────────────────────────────────────

    fn run(&mut self, movie: &Movie) -> game_core::Result<()> {
        for scene in &movie.scenes {
            // Clear screen before each scene
            let mut out = io::stdout();
            execute!(out, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))
                .map_err(|e| GameError::Io(e.to_string()))?;

            self.draw_status()?;

            match self.play_scene(scene)? {
                Playback::Quit => break,
                Playback::SkipScene | Playback::Continue => continue,
            }
        }
        Ok(())
    }

    fn play_scene(&mut self, scene: &super::types::Scene) -> game_core::Result<Playback> {
        for act in &scene.acts {
            match self.exec(act)? {
                Playback::Quit => return Ok(Playback::Quit),
                Playback::SkipScene => return Ok(Playback::SkipScene),
                Playback::Continue => {}
            }
        }
        Ok(Playback::Continue)
    }

    // ── Act execution ──────────────────────────────────────────

    fn exec(&mut self, act: &Act) -> game_core::Result<Playback> {
        match act {
            Act::Type {
                row,
                col,
                text,
                ms_per_word,
                style,
            } => self.typewriter(*row, *col, text, *ms_per_word, style),

            Act::Center {
                row,
                text,
                ms_per_word,
                style,
            } => {
                let col = self.center_col(text.len() as u16);
                self.typewriter(*row, col, text, *ms_per_word, style)
            }

            Act::Print {
                row,
                col,
                text,
                style,
            } => {
                let mut out = io::stdout();
                queue!(out, cursor::MoveTo(*col, *row))
                    .map_err(|e| GameError::Io(e.to_string()))?;
                self.apply_style(&mut out, style)?;
                queue!(out, Print(text)).map_err(|e| GameError::Io(e.to_string()))?;
                self.reset_style(&mut out)?;
                out.flush().map_err(|e| GameError::Io(e.to_string()))?;
                Ok(Playback::Continue)
            }

            Act::Wait(ms) => {
                let adjusted = ((*ms as f64) * self.speed) as u64;
                match self.sleep(adjusted)? {
                    Signal::Quit => Ok(Playback::Quit),
                    Signal::SkipScene => Ok(Playback::SkipScene),
                    _ => Ok(Playback::Continue),
                }
            }

            Act::Fade(ms) => {
                let adjusted = ((*ms as f64) * self.speed) as u64;
                let first = adjusted
                    .checked_mul(2)
                    .unwrap_or(adjusted)
                    .checked_div(5)
                    .unwrap_or(0);
                let second = adjusted.saturating_sub(first);

                match self.sleep(first)? {
                    Signal::Quit => return Ok(Playback::Quit),
                    Signal::SkipScene => return Ok(Playback::SkipScene),
                    _ => {}
                }

                let mut out = io::stdout();
                execute!(out, terminal::Clear(ClearType::All))
                    .map_err(|e| GameError::Io(e.to_string()))?;

                match self.sleep(second)? {
                    Signal::Quit => Ok(Playback::Quit),
                    Signal::SkipScene => Ok(Playback::SkipScene),
                    _ => Ok(Playback::Continue),
                }
            }
        }
    }

    // ── Typewriter effect ──────────────────────────────────────

    fn typewriter(
        &mut self,
        row: u16,
        start_col: u16,
        text: &str,
        ms_per_word: u64,
        style: &Style,
    ) -> game_core::Result<Playback> {
        let mut out = io::stdout();
        let mut col = start_col;

        for (i, word) in text.split_whitespace().enumerate() {
            // Add space between words
            if i > 0 {
                queue!(out, cursor::MoveTo(col, row), Print(" "))
                    .map_err(|e| GameError::Io(e.to_string()))?;
                col = col.saturating_add(1);
            }

            // Position, style, print word
            queue!(out, cursor::MoveTo(col, row)).map_err(|e| GameError::Io(e.to_string()))?;
            self.apply_style(&mut out, style)?;
            queue!(out, Print(word)).map_err(|e| GameError::Io(e.to_string()))?;
            self.reset_style(&mut out)?;
            out.flush().map_err(|e| GameError::Io(e.to_string()))?;

            col = col.saturating_add(word.len() as u16);

            // Delay between words (skip for ms_per_word == 0)
            if ms_per_word > 0 {
                let adjusted = ((ms_per_word as f64) * self.speed) as u64;
                match self.sleep(adjusted)? {
                    Signal::Quit => return Ok(Playback::Quit),
                    Signal::SkipScene => return Ok(Playback::SkipScene),
                    _ => {}
                }
            }
        }

        Ok(Playback::Continue)
    }

    // ── Centering ──────────────────────────────────────────────

    fn center_col(&self, text_len: u16) -> u16 {
        if self.term_width > text_len {
            (self.term_width.saturating_sub(text_len))
                .checked_div(2)
                .unwrap_or(0)
        } else {
            0
        }
    }

    // ── Interruptible sleep ────────────────────────────────────

    fn sleep(&mut self, ms: u64) -> game_core::Result<Signal> {
        let chunk = 50u64;
        let mut remaining = ms;

        while remaining > 0 {
            let wait = remaining.min(chunk);
            let polled = event::poll(Duration::from_millis(wait))
                .map_err(|e| GameError::Io(e.to_string()))?;

            if polled {
                match self.poll()? {
                    Signal::None => {}
                    Signal::Quit => return Ok(Signal::Quit),
                    Signal::SkipScene => return Ok(Signal::SkipScene),
                    Signal::Pause => {
                        // Block until unpause or quit
                        match self.wait_unpause()? {
                            Signal::Quit => return Ok(Signal::Quit),
                            Signal::SkipScene => return Ok(Signal::SkipScene),
                            _ => {}
                        }
                    }
                    Signal::Faster => {
                        self.speed = (self.speed - 0.25).max(0.25);
                        self.draw_status()?;
                    }
                    Signal::Slower => {
                        self.speed = (self.speed + 0.25).min(4.0);
                        self.draw_status()?;
                    }
                }
            }

            remaining = remaining.saturating_sub(wait);
        }

        Ok(Signal::None)
    }

    /// Block until user unpauses or quits.
    fn wait_unpause(&mut self) -> game_core::Result<Signal> {
        // Show paused indicator
        self.draw_status_paused()?;

        loop {
            let ev = event::read().map_err(|e| GameError::Io(e.to_string()))?;
            if let Event::Key(key) = ev {
                match key.code {
                    KeyCode::Char(' ') => {
                        // Unpause — redraw normal status
                        self.draw_status()?;
                        return Ok(Signal::None);
                    }
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(Signal::Quit),
                    KeyCode::Right => return Ok(Signal::SkipScene),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(Signal::Quit);
                    }
                    _ => {}
                }
            }
        }
    }

    // ── Input polling ──────────────────────────────────────────

    fn poll(&self) -> game_core::Result<Signal> {
        let ev = event::read().map_err(|e| GameError::Io(e.to_string()))?;
        if let Event::Key(key) = ev {
            // Ctrl+C always quits
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                if let KeyCode::Char('c') = key.code {
                    return Ok(Signal::Quit);
                }
            }
            return Ok(match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Signal::Quit,
                KeyCode::Char(' ') => Signal::Pause,
                KeyCode::Right => Signal::SkipScene,
                KeyCode::Char('+') | KeyCode::Char('=') => Signal::Faster,
                KeyCode::Char('-') => Signal::Slower,
                _ => Signal::None,
            });
        }
        Ok(Signal::None)
    }

    // ── Styling ────────────────────────────────────────────────

    fn apply_style(&self, out: &mut io::Stdout, style: &Style) -> game_core::Result<()> {
        if style.bold {
            queue!(out, SetAttribute(Attribute::Bold)).map_err(|e| GameError::Io(e.to_string()))?;
        }
        if style.dim {
            queue!(out, SetAttribute(Attribute::Dim)).map_err(|e| GameError::Io(e.to_string()))?;
        }
        if style.italic {
            queue!(out, SetAttribute(Attribute::Italic))
                .map_err(|e| GameError::Io(e.to_string()))?;
        }
        if let Some(color) = &style.color {
            queue!(out, SetForegroundColor(Self::map_color(*color)))
                .map_err(|e| GameError::Io(e.to_string()))?;
        }
        Ok(())
    }

    fn reset_style(&self, out: &mut io::Stdout) -> game_core::Result<()> {
        queue!(out, SetAttribute(Attribute::Reset), ResetColor)
            .map_err(|e| GameError::Io(e.to_string()))?;
        Ok(())
    }

    fn map_color(color: Color) -> CtColor {
        match color {
            Color::Yellow => CtColor::Yellow,
            Color::Cyan => CtColor::Cyan,
            Color::Green => CtColor::Green,
            Color::Red => CtColor::Red,
            Color::Magenta => CtColor::Magenta,
            Color::White => CtColor::White,
            Color::Grey => CtColor::DarkGrey,
        }
    }

    // ── Status line ────────────────────────────────────────────

    fn draw_status(&self) -> game_core::Result<()> {
        let mut out = io::stdout();
        let speed_pct = (100.0 / self.speed) as u32;

        queue!(
            out,
            cursor::MoveTo(2, self.status_row),
            SetAttribute(Attribute::Dim),
            SetForegroundColor(CtColor::DarkGrey),
            Print(obfstr!(
                "[q] quit  [space] pause  [\u{2192}] skip  [+/-] speed"
            )),
            Print(format!("  {}%", speed_pct)),
            SetAttribute(Attribute::Reset),
            ResetColor
        )
        .map_err(|e| GameError::Io(e.to_string()))?;

        out.flush().map_err(|e| GameError::Io(e.to_string()))?;
        Ok(())
    }

    fn draw_status_paused(&self) -> game_core::Result<()> {
        let mut out = io::stdout();

        queue!(
            out,
            cursor::MoveTo(2, self.status_row),
            SetAttribute(Attribute::Dim),
            SetForegroundColor(CtColor::Yellow),
            Print(obfstr!("PAUSED  [space] resume  [q] quit  [\u{2192}] skip")),
            Print("                    "),
            SetAttribute(Attribute::Reset),
            ResetColor
        )
        .map_err(|e| GameError::Io(e.to_string()))?;

        out.flush().map_err(|e| GameError::Io(e.to_string()))?;
        Ok(())
    }
}
