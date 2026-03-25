use crate::engine::{Direction, Entity, EntityType, Input, Map, Session, Tile, Universe};
use crate::games::{Game, GameAction};

pub struct Lobby {
    universe: Universe,
    session: Session,
    player_x: usize,
    player_y: usize,
}

// Define where game portals are
const POINTSET_X: usize = 10;
const POINTSET_Y: usize = 7;
const TETRIS_X: usize = 30;
const TETRIS_Y: usize = 7;
const SNAKE_X: usize = 10;
const SNAKE_Y: usize = 14;
const INVADERS_X: usize = 30;
const INVADERS_Y: usize = 14;

impl Game for Lobby {
    fn new(mut universe: Universe) -> Self {
        // Place game icons as walkable entities
        let _ = universe.map.set_tile(
            POINTSET_X,
            POINTSET_Y,
            Tile::Entity(Entity {
                id: "pointset".to_string(),
                entity_type: EntityType::Item,
                display: 'P',
            }),
        );

        let _ = universe.map.set_tile(
            TETRIS_X,
            TETRIS_Y,
            Tile::Entity(Entity {
                id: "tetris".to_string(),
                entity_type: EntityType::Item,
                display: 'T',
            }),
        );

        let _ = universe.map.set_tile(
            SNAKE_X,
            SNAKE_Y,
            Tile::Entity(Entity {
                id: "snake".to_string(),
                entity_type: EntityType::Item,
                display: 'S',
            }),
        );

        let _ = universe.map.set_tile(
            INVADERS_X,
            INVADERS_Y,
            Tile::Entity(Entity {
                id: "invaders".to_string(),
                entity_type: EntityType::Item,
                display: 'I',
            }),
        );

        // Start player in center
        let start_x = universe.map.width / 2;
        let start_y = universe.map.height / 2;

        Self {
            universe,
            session: Session::new("game".to_string()),
            player_x: start_x,
            player_y: start_y,
        }
    }

    fn tick(&mut self, input: Input) -> GameAction {
        match input {
            Input::Move(direction) => {
                let (new_x, new_y) = match direction {
                    Direction::Up => (self.player_x, self.player_y.saturating_sub(1)),
                    Direction::Down => (
                        self.player_x,
                        (self.player_y + 1).min(self.universe.map.height - 1),
                    ),
                    Direction::Left => (self.player_x.saturating_sub(1), self.player_y),
                    Direction::Right => (
                        (self.player_x + 1).min(self.universe.map.width - 1),
                        self.player_y,
                    ),
                };

                // Check if we can move there
                match self.universe.map.get_tile(new_x, new_y) {
                    Some(Tile::Wall) => {
                        // Can't move into walls
                        GameAction::Continue
                    }
                    _ => {
                        // Move is valid
                        self.player_x = new_x;
                        self.player_y = new_y;

                        // Check if we stepped on a game icon
                        if new_x == POINTSET_X && new_y == POINTSET_Y {
                            GameAction::Teleport("pointset".to_string())
                        } else if new_x == TETRIS_X && new_y == TETRIS_Y {
                            GameAction::Teleport("tetris".to_string())
                        } else if new_x == SNAKE_X && new_y == SNAKE_Y {
                            GameAction::Teleport("snake".to_string())
                        } else if new_x == INVADERS_X && new_y == INVADERS_Y {
                            GameAction::Teleport("invaders".to_string())
                        } else {
                            GameAction::Continue
                        }
                    }
                }
            }
            Input::Quit => GameAction::Quit,
            Input::Action => GameAction::Continue,
            Input::None => GameAction::Continue,
            // Poker-specific inputs are not used in lobby
            _ => GameAction::Continue,
        }
    }

    fn render(&self) -> Map {
        let mut map = self.universe.map.clone();

        // Draw player at current position
        let _ = map.set_tile(
            self.player_x,
            self.player_y,
            Tile::Entity(Entity {
                id: "player".to_string(),
                entity_type: EntityType::Player,
                display: '@',
            }),
        );

        map
    }

    fn get_session(&self) -> &Session {
        &self.session
    }

    fn get_universe(&self) -> &Universe {
        &self.universe
    }
}
