use colored::Colorize;
use crossterm::{
    cursor, execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use crate::engine::universe::{Map, Tile};

#[derive(Clone, Debug)]
pub struct Cell {
    pub character: char,
    pub color: CellColor,
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum CellColor {
    Default,
    Grid,
    Wall,
    Portal,
    Player,
    Entity,
    GameIcon,
}

pub struct Renderer {
    width: usize,
    height: usize,
    buffer: Vec<Vec<Cell>>,
}

impl Renderer {
    pub fn new(width: usize, height: usize) -> Self {
        let buffer = vec![
            vec![
                Cell {
                    character: ' ',
                    color: CellColor::Default,
                };
                width
            ];
            height
        ];

        Self {
            width,
            height,
            buffer,
        }
    }

    pub fn clear(&mut self) {
        for row in &mut self.buffer {
            for cell in row {
                cell.character = ' ';
                cell.color = CellColor::Default;
            }
        }
    }

    pub fn draw_cell(&mut self, x: usize, y: usize, cell: Cell) {
        if x < self.width && y < self.height {
            if let Some(row) = self.buffer.get_mut(y) {
                if let Some(c) = row.get_mut(x) {
                    *c = cell;
                }
            }
        }
    }

    pub fn draw_map(&mut self, map: &Map) {
        self.clear();

        for y in 0..map.height {
            for x in 0..map.width {
                if let Some(tile) = map.get_tile(x, y) {
                    let (character, color) = match tile {
                        Tile::Empty => (' ', CellColor::Default),
                        Tile::Wall => ('█', CellColor::Wall),
                        Tile::Portal(target) => {
                            let c = if target.contains("pointset") {
                                'P'
                            } else if target.contains("tetris") {
                                'T'
                            } else if target.contains("snake") {
                                'S'
                            } else if target.contains("invaders") {
                                'I'
                            } else {
                                '◇'
                            };
                            (c, CellColor::Portal)
                        }
                        Tile::Entity(entity) => (entity.display, CellColor::Entity),
                    };

                    self.draw_cell(x, y, Cell { character, color });
                }
            }
        }
    }

    pub fn draw_text(&mut self, x: usize, y: usize, text: &str, color: CellColor) {
        for (i, ch) in text.chars().enumerate() {
            if x + i < self.width && y < self.height {
                self.draw_cell(
                    x + i,
                    y,
                    Cell {
                        character: ch,
                        color,
                    },
                );
            }
        }
    }

    pub fn draw_grid(&mut self) {
        // Grid disabled - kept for compatibility but does nothing
        // The map tiles provide all needed visual guidance
    }

    pub fn flush(&self) -> io::Result<()> {
        let mut stdout = io::stdout();

        execute!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        for row in &self.buffer {
            for cell in row {
                let ch = cell.character.to_string();
                let colored = match cell.color {
                    CellColor::Wall => ch.bright_blue(),
                    CellColor::Portal => ch.yellow(),
                    CellColor::Player => ch.bright_green().bold(),
                    CellColor::Entity => ch.magenta(),
                    CellColor::GameIcon => ch.bright_yellow().bold(),
                    CellColor::Grid => ch.bright_black(),
                    CellColor::Default => ch.normal(),
                };
                write!(stdout, "{}", colored)?;
            }
            writeln!(stdout)?;
        }

        stdout.flush()?;
        Ok(())
    }
}

pub fn setup_terminal() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), terminal::EnterAlternateScreen)?;
    Ok(())
}

pub fn cleanup_terminal() -> io::Result<()> {
    execute!(io::stdout(), terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}
