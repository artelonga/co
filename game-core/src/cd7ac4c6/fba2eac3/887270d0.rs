use super::{BettingRound, GameConfig, PlayerStatus, PokerGame, SelectedAction};
use colored::Colorize;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};
use std::time::Duration;

/// Terminal dimensions for rendering
#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

impl TerminalSize {
    /// Get current terminal size
    pub fn current() -> Self {
        let (width, height) = terminal::size().unwrap_or((80, 24));
        Self { width, height }
    }

    /// Check if terminal is large enough for the given config
    pub fn is_valid_for(&self, config: &GameConfig) -> bool {
        self.width >= config.min_width && self.height >= config.min_height
    }

    /// Get effective width for table, respecting config and terminal bounds
    pub fn table_width_for(&self, config: &GameConfig) -> usize {
        // Use config width, but cap at terminal width
        let config_width = config.width as usize;
        let term_width = (self.width as usize).saturating_sub(2);
        config_width.min(term_width).max(config.min_width as usize)
    }

    /// Get effective height, respecting config and terminal bounds
    pub fn table_height_for(&self, config: &GameConfig) -> usize {
        let config_height = config.height as usize;
        let term_height = self.height as usize;
        config_height
            .min(term_height)
            .max(config.min_height as usize)
    }
}

/// Check for terminal resize event
pub fn check_resize() -> Option<TerminalSize> {
    if event::poll(Duration::from_millis(0)).unwrap_or(false) {
        if let Ok(Event::Resize(w, h)) = event::read() {
            return Some(TerminalSize {
                width: w,
                height: h,
            });
        }
    }
    None
}

/// Render the poker game to the terminal with config-based sizing
pub fn render_poker_game(game: &PokerGame) {
    let term_size = TerminalSize::current();
    render_poker_game_sized(game, term_size);
}

/// Render with explicit terminal size, using game config for dimensions
pub fn render_poker_game_sized(game: &PokerGame, term_size: TerminalSize) {
    let mut stdout = io::stdout();
    let config = &game.config;

    // Robust screen clear for resize handling:
    // 1. Purge scrollback buffer
    // 2. Clear entire screen
    // 3. Move cursor to top-left
    // 4. Hide cursor during render
    let _ = execute!(
        stdout,
        terminal::Clear(ClearType::Purge),
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Hide
    );
    let _ = stdout.flush();

    // Check if terminal is too small for minimum config requirements
    if !term_size.is_valid_for(config) {
        println!(
            "{}",
            format!(
                "Terminal too small! Need at least {}x{}, got {}x{}",
                config.min_width, config.min_height, term_size.width, term_size.height
            )
            .bright_red()
        );
        println!("Please resize your terminal window.");
        let _ = stdout.flush();
        return;
    }

    // Get effective dimensions from config, bounded by terminal
    let table_width = term_size.table_width_for(config);

    // Header
    print_header(game, table_width);

    // Community cards
    print_community_cards(game, table_width);

    // Players
    print_players(game, table_width);

    // Local player's cards
    print_hole_cards(game, table_width);

    // Actions or status
    if game.is_local_turn() {
        print_actions(game, table_width);
    } else if game.game_over {
        print_game_over(game, table_width);
    } else {
        print_waiting(game, table_width);
    }

    // Chat (only if we have room)
    if term_size.height >= 24 {
        print_chat(game, table_width);
    }

    // Footer with controls hint
    print_footer(table_width);

    let _ = execute!(stdout, cursor::Show);
    let _ = stdout.flush();
}

fn print_header(game: &PokerGame, width: usize) {
    let border = "═".repeat(width);
    println!("{}", border.bright_blue());

    // Build header content
    let room_str = format!("Room: {}", game.room_id);
    let pot_str = format!("Pot: ${}", game.table.pot);
    let blind_str = format!(
        "Blinds: ${}/{}",
        game.table.small_blind, game.table.big_blind
    );

    // Calculate spacing
    let content_len = room_str.len() + pot_str.len() + blind_str.len();
    let available_space = width.saturating_sub(content_len + 6);
    let spacing = available_space / 2;

    print!("║ ");
    print!("{}", room_str.bright_cyan());
    print!("{}", " ".repeat(spacing.max(2)));
    print!("{}", pot_str.bright_yellow().bold());
    print!("{}", " ".repeat(spacing.max(2)));
    print!("{}", blind_str);

    // Pad to width
    let printed = 3 + room_str.len() + spacing + pot_str.len() + spacing + blind_str.len();
    if printed < width - 1 {
        print!("{}", " ".repeat(width - printed - 1));
    }
    println!("║");

    println!("{}", border.bright_blue());
}

fn print_community_cards(game: &PokerGame, width: usize) {
    let cards_display = game.table.display_community(game.round);
    let round_name = game.round.name();

    println!();

    // Center the cards
    let cards_visual_len = 24; // 5 cards * ~5 chars each with spacing
    let padding = (width.saturating_sub(cards_visual_len)) / 2;

    println!("{}{}", " ".repeat(padding), cards_display);

    let label = format!("Community Cards ({})", round_name);
    let label_padding = (width.saturating_sub(label.len())) / 2;
    println!(
        "{}Community Cards ({})",
        " ".repeat(label_padding),
        round_name.bright_cyan()
    );
    println!();
}

fn print_players(game: &PokerGame, width: usize) {
    println!("{}", "─".repeat(width).bright_black());

    for (i, player) in game.players.iter().enumerate() {
        let is_action = i == game.table.action_position && !game.game_over;
        let is_local = i == game.local_player_index;

        // Build player line
        let mut line = String::new();

        // Turn indicator
        if is_action {
            line.push_str(&"► ".bright_green().to_string());
        } else {
            line.push_str("  ");
        }

        // Player name
        let name = if is_local {
            player.name.bright_green().bold().to_string()
        } else {
            player.name.clone()
        };
        line.push_str(&name);

        // Dealer button
        if player.is_dealer {
            line.push_str(&" [D]".bright_yellow().to_string());
        }

        // Chips
        line.push_str(&format!(": ${}", player.chips.to_string().bright_yellow()));

        // Current bet
        if player.current_bet > 0 {
            line.push_str(
                &format!(" (bet: ${})", player.current_bet)
                    .bright_black()
                    .to_string(),
            );
        }

        // Status
        let status = match player.status {
            PlayerStatus::Active => "",
            PlayerStatus::Folded => " (folded)",
            PlayerStatus::AllIn => " (ALL IN)",
            PlayerStatus::Waiting => " (waiting)",
            PlayerStatus::SittingOut => " (sitting out)",
        };

        if !status.is_empty() {
            let styled = match player.status {
                PlayerStatus::AllIn => status.bright_red().to_string(),
                _ => status.bright_black().to_string(),
            };
            line.push_str(&styled);
        }

        println!("  {}", line);
    }

    println!("{}", "─".repeat(width).bright_black());
}

fn print_hole_cards(game: &PokerGame, _width: usize) {
    let local_player = crate::engine::at(&game.players, game.local_player_index);

    println!();
    if let Some(cards) = &local_player.hole_cards {
        let card1 = crate::engine::at(cards, 0).display_bracketed();
        let card2 = crate::engine::at(cards, 1).display_bracketed();
        println!("  Your cards:  {}  {}", card1, card2);
    } else {
        println!("  Your cards:  [  ]  [  ]");
    }
    println!();
}

fn print_actions(game: &PokerGame, _width: usize) {
    let available = game.available_actions();

    print!("  Actions: ");

    for action in SelectedAction::all() {
        let is_selected = action == game.selected_action;
        let is_available = available.contains(&action);

        let label = action.label();

        if is_selected && is_available {
            print!(" {} ", label.black().on_bright_green());
        } else if is_available {
            print!(" {} ", label);
        } else {
            print!(" {} ", label.bright_black());
        }
    }
    println!();

    // Show raise amount if raise is selected
    if game.selected_action == SelectedAction::Raise {
        println!(
            "  Raise amount: ${} (↑/↓ to adjust)",
            game.raise_amount.to_string().bright_yellow()
        );
    }

    println!();
    println!("  Press {} to confirm action", "Enter".bright_green());
}

fn print_game_over(game: &PokerGame, _width: usize) {
    if let Some(msg) = &game.winner_message {
        println!("  {}", msg.bright_yellow().bold());
    }

    // Show all hands at showdown
    if game.round == BettingRound::Showdown {
        println!();
        for player in &game.players {
            if let Some(cards) = &player.hole_cards {
                if matches!(player.status, PlayerStatus::Active | PlayerStatus::AllIn) {
                    println!(
                        "  {}: {} {}",
                        player.name,
                        crate::engine::at(cards, 0).display_bracketed(),
                        crate::engine::at(cards, 1).display_bracketed()
                    );
                }
            }
        }
    }

    println!();
    println!("  Press {} to start new hand", "Enter".bright_green());
}

fn print_waiting(game: &PokerGame, _width: usize) {
    let current_player = crate::engine::at(&game.players, game.table.action_position);
    println!(
        "  Waiting for {} to act...",
        current_player.name.bright_cyan()
    );
    println!();
}

fn print_chat(game: &PokerGame, width: usize) {
    println!("{}", "─".repeat(width).bright_black());
    println!("  Chat:");

    let messages_to_show: Vec<_> = game.chat_messages.iter().rev().take(3).collect();

    if messages_to_show.is_empty() {
        println!("    {}", "(no messages)".bright_black());
    } else {
        for msg in messages_to_show.iter().rev() {
            // Truncate message if needed
            let max_msg_len = width.saturating_sub(msg.sender.len() + 8);
            let content = if msg.content.len() > max_msg_len {
                let end = max_msg_len.saturating_sub(3);
                let truncated = msg.content.get(..end).unwrap_or(&msg.content);
                format!("{}...", truncated)
            } else {
                msg.content.clone()
            };
            println!("    {}: {}", msg.sender.bright_cyan(), content);
        }
    }

    println!("{}", "─".repeat(width).bright_black());
}

fn print_footer(width: usize) {
    println!();
    let help = "F:Fold  C:Check  K:Call  R:Raise  A:All-in  Q:Quit";
    let padding = (width.saturating_sub(help.len())) / 2;
    println!("{}{}", " ".repeat(padding), help.bright_black());
}

/// Render a simple card for display in other contexts
pub fn render_card_simple(rank: &str, suit: &str, is_red: bool) -> String {
    if is_red {
        format!("\x1b[31m{}{}\x1b[0m", rank, suit)
    } else {
        format!("{}{}", rank, suit)
    }
}

/// Render a card in box format
pub fn render_card_box(rank: &str, suit: &str, is_red: bool) -> String {
    let content = if is_red {
        format!("\x1b[31m{}{}\x1b[0m", rank, suit)
    } else {
        format!("{}{}", rank, suit)
    };

    format!("[{}]", content)
}

/// Poll for input including resize events
/// Returns (input, Option<new_size>)
pub fn poll_input_with_resize() -> crate::engine::Result<(crate::Input, Option<TerminalSize>)> {
    use crate::Input;

    // Drain all pending events to get the most recent state
    let mut last_resize: Option<TerminalSize> = None;
    let mut last_input: Option<Input> = None;

    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            Event::Resize(w, h) => {
                // Keep track of the latest resize dimensions
                last_resize = Some(TerminalSize {
                    width: w,
                    height: h,
                });
            }
            Event::Key(key) => {
                use crossterm::event::KeyModifiers;

                // Check for Ctrl+C
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    if let KeyCode::Char('c') = key.code {
                        last_input = Some(Input::Quit);
                        continue;
                    }
                }

                last_input = Some(match key.code {
                    KeyCode::Up => Input::Move(crate::Direction::Up),
                    KeyCode::Down => Input::Move(crate::Direction::Down),
                    KeyCode::Left => Input::Move(crate::Direction::Left),
                    KeyCode::Right => Input::Move(crate::Direction::Right),
                    KeyCode::Enter => Input::Action,
                    KeyCode::Esc => Input::Quit,
                    KeyCode::Char('f') | KeyCode::Char('F') => Input::Fold,
                    KeyCode::Char('c') | KeyCode::Char('C') => Input::Check,
                    KeyCode::Char('k') | KeyCode::Char('K') => Input::Call,
                    KeyCode::Char('r') | KeyCode::Char('R') => Input::Raise,
                    KeyCode::Char('a') | KeyCode::Char('A') => Input::AllIn,
                    KeyCode::Char('q') | KeyCode::Char('Q') => Input::Quit,
                    KeyCode::Char(c @ '0'..='9') => Input::Number((c as u8) - b'0'),
                    KeyCode::Char(c) => Input::Char(c),
                    KeyCode::Backspace => Input::Backspace,
                    _ => Input::None,
                });
            }
            _ => {}
        }
    }

    // If no events were ready, do a brief poll to avoid busy-spinning
    if last_resize.is_none() && last_input.is_none() && event::poll(Duration::from_millis(16))? {
        match event::read()? {
            Event::Resize(w, h) => {
                last_resize = Some(TerminalSize {
                    width: w,
                    height: h,
                });
            }
            Event::Key(key) => {
                use crossterm::event::KeyModifiers;

                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    if let KeyCode::Char('c') = key.code {
                        return Ok((Input::Quit, None));
                    }
                }

                let input = match key.code {
                    KeyCode::Up => Input::Move(crate::Direction::Up),
                    KeyCode::Down => Input::Move(crate::Direction::Down),
                    KeyCode::Left => Input::Move(crate::Direction::Left),
                    KeyCode::Right => Input::Move(crate::Direction::Right),
                    KeyCode::Enter => Input::Action,
                    KeyCode::Esc => Input::Quit,
                    KeyCode::Char('f') | KeyCode::Char('F') => Input::Fold,
                    KeyCode::Char('c') | KeyCode::Char('C') => Input::Check,
                    KeyCode::Char('k') | KeyCode::Char('K') => Input::Call,
                    KeyCode::Char('r') | KeyCode::Char('R') => Input::Raise,
                    KeyCode::Char('a') | KeyCode::Char('A') => Input::AllIn,
                    KeyCode::Char('q') | KeyCode::Char('Q') => Input::Quit,
                    KeyCode::Char(c @ '0'..='9') => Input::Number((c as u8) - b'0'),
                    KeyCode::Char(c) => Input::Char(c),
                    KeyCode::Backspace => Input::Backspace,
                    _ => Input::None,
                };
                return Ok((input, None));
            }
            _ => {}
        }
    }

    Ok((last_input.unwrap_or(Input::None), last_resize))
}
