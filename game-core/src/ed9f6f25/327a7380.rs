use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Universe {
    pub name: String,
    pub map: Map,
    pub objective: Objective,
    pub rules: Rules,
    pub session_config: SessionConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Map {
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<Vec<Tile>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Tile {
    Empty,
    Wall,
    Portal(String),
    Entity(Entity),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub entity_type: EntityType,
    pub display: char,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EntityType {
    Player,
    NPC,
    Obstacle,
    Item,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Portal {
    pub position: (usize, usize),
    pub target_universe: String,
    pub display: char,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Objective {
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rules {
    pub allow_movement: bool,
    pub allow_teleport: bool,
    pub max_width: usize,
    pub max_height: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub timeout_seconds: u64,
    pub save_history: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            allow_movement: true,
            allow_teleport: true,
            max_width: 80,
            max_height: 24,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 3600,
            save_history: true,
        }
    }
}

impl Default for Objective {
    fn default() -> Self {
        Self {
            description: "Explore and navigate".to_string(),
        }
    }
}

impl Map {
    pub fn new(width: usize, height: usize) -> Self {
        let mut tiles = vec![vec![Tile::Empty; width]; height];

        // Add borders
        for y in 0..height {
            for x in 0..width {
                if x == 0 || x == width - 1 || y == 0 || y == height - 1 {
                    if let Some(row) = tiles.get_mut(y) {
                        if let Some(cell) = row.get_mut(x) {
                            *cell = Tile::Wall;
                        }
                    }
                }
            }
        }

        Self {
            width,
            height,
            tiles,
        }
    }

    pub fn set_tile(&mut self, x: usize, y: usize, tile: Tile) -> crate::engine::Result<()> {
        if x >= self.width || y >= self.height {
            return Err(crate::engine::GameError::InvalidState(format!(
                "Position ({}, {}) out of bounds",
                x, y
            )));
        }
        if let Some(row) = self.tiles.get_mut(y) {
            if let Some(cell) = row.get_mut(x) {
                *cell = tile;
            }
        }
        Ok(())
    }

    pub fn get_tile(&self, x: usize, y: usize) -> Option<&Tile> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.tiles.get(y).and_then(|row| row.get(x))
    }

    pub fn add_portal(&mut self, x: usize, y: usize, target: String) -> crate::engine::Result<()> {
        self.set_tile(x, y, Tile::Portal(target))
    }
}

impl Universe {
    pub fn new(name: String, width: usize, height: usize) -> Self {
        Self {
            name,
            map: Map::new(width, height),
            objective: Objective::default(),
            rules: Rules::default(),
            session_config: SessionConfig::default(),
        }
    }

    pub fn lobby() -> Self {
        let mut universe = Self::new("game".to_string(), 40, 20);
        universe.objective = Objective {
            description: "Navigate to a game portal".to_string(),
        };

        // Add portals to games
        let _ = universe
            .map
            .set_tile(10, 8, Tile::Portal("pointset".to_string()));
        let _ = universe
            .map
            .set_tile(20, 8, Tile::Portal("tetris".to_string()));
        let _ = universe
            .map
            .set_tile(10, 12, Tile::Portal("snake".to_string()));
        let _ = universe
            .map
            .set_tile(20, 12, Tile::Portal("invaders".to_string()));

        universe
    }

    pub fn pointset() -> Self {
        Self::new("pointset".to_string(), 40, 20)
    }

    pub fn tetris() -> Self {
        Self::new("tetris".to_string(), 20, 24)
    }

    pub fn snake() -> Self {
        Self::new("snake".to_string(), 40, 20)
    }

    pub fn invaders() -> Self {
        Self::new("invaders".to_string(), 60, 20)
    }
}
