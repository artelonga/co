use crate::engine::{Input, Map, Session, Universe};
use crate::games::{Game, GameAction};

pub struct PointSetGame {
    universe: Universe,
    session: Session,
}

impl Game for PointSetGame {
    fn new(universe: Universe) -> Self {
        Self {
            universe,
            session: Session::new("pointset".to_string()),
        }
    }

    fn tick(&mut self, input: Input) -> GameAction {
        match input {
            Input::Quit => GameAction::Teleport("game".to_string()),
            _ => GameAction::Continue,
        }
    }

    fn render(&self) -> Map {
        self.universe.map.clone()
    }

    fn get_session(&self) -> &Session {
        &self.session
    }

    fn get_universe(&self) -> &Universe {
        &self.universe
    }
}
