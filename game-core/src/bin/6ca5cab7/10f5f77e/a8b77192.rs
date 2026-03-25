use game_core::Result;
use game_core::{Game, Lobby, Universe};
use serde_json::json;

pub fn run_query(x: usize, y: usize, move_dir: Option<String>) -> Result<()> {
    // Create lobby game with default size
    let mut lobby = Lobby::new(Universe::lobby());

    // Apply move if provided
    if let Some(dir) = move_dir {
        let input = match dir.as_str() {
            "up" => game_core::Input::Move(game_core::Direction::Up),
            "down" => game_core::Input::Move(game_core::Direction::Down),
            "left" => game_core::Input::Move(game_core::Direction::Left),
            "right" => game_core::Input::Move(game_core::Direction::Right),
            "q" => game_core::Input::Quit,
            _ => game_core::Input::None,
        };

        let _action = lobby.tick(input);
    }

    // Render current state
    let map = lobby.render();

    // Convert to JSON for easy parsing
    let mut board = vec![];
    for row in 0..map.height {
        let mut line = String::new();
        for col in 0..map.width {
            if let Some(tile) = map.get_tile(col, row) {
                let ch = match tile {
                    game_core::Tile::Empty => ' ',
                    game_core::Tile::Wall => '█',
                    game_core::Tile::Portal(target) => {
                        if target.contains("pointset") {
                            'P'
                        } else if target.contains("tetris") {
                            'T'
                        } else if target.contains("snake") {
                            'S'
                        } else if target.contains("invaders") {
                            'I'
                        } else {
                            '?'
                        }
                    }
                    game_core::Tile::Entity(entity) => entity.display,
                };
                line.push(ch);
            }
        }
        board.push(line);
    }

    // Output as JSON
    let output = json!({
        "width": map.width,
        "height": map.height,
        "board": board,
        "status": "ok"
    });

    println!("{}", output.to_string());
    Ok(())
}
