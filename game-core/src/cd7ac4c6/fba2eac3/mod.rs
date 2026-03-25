#[path = "d8303259.rs"]
pub mod deck;
#[path = "1b001706.rs"]
pub mod hand;
#[path = "887270d0.rs"]
pub mod render;

use crate::engine::{at, at_mut, Direction, Input, Interpreter, Map, Session, Universe, Value};
use crate::games::GameAction;
use crate::storage::schema as proto;
use crate::storage::{self, HandRecorder, Storage};
use deck::{Card, Deck};
use hand::{evaluate_hand, EvaluatedHand};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

pub use deck::{Rank, Suit};
pub use hand::compare_hands;

/// Game configuration loaded from script file
#[derive(Clone, Debug)]
pub struct GameConfig {
    /// Display width (characters)
    pub width: u16,
    /// Display height (characters)
    pub height: u16,
    /// Minimum width
    pub min_width: u16,
    /// Minimum height
    pub min_height: u16,
    /// Small blind amount
    pub small_blind: u32,
    /// Big blind amount
    pub big_blind: u32,
    /// Starting chips
    pub starting_chips: u32,
    /// Maximum players
    pub max_players: usize,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            width: 60,
            height: 24,
            min_width: 50,
            min_height: 20,
            small_blind: 10,
            big_blind: 20,
            starting_chips: 1000,
            max_players: 6,
        }
    }
}

impl GameConfig {
    /// Load configuration from encrypted database
    pub fn from_storage(storage: &Storage) -> Self {
        match storage.get_game_config(obfstr::obfstr!("poker")) {
            Ok(Some(proto_config)) => Self::from_proto(&proto_config),
            _ => Self::default(),
        }
    }

    /// Convert from protobuf GameConfig
    fn from_proto(proto: &proto::GameConfig) -> Self {
        Self {
            width: proto.width as u16,
            height: proto.height as u16,
            min_width: proto.min_width as u16,
            min_height: proto.min_height as u16,
            small_blind: proto.small_blind,
            big_blind: proto.big_blind,
            starting_chips: proto.starting_chips,
            max_players: proto.max_players as usize,
        }
    }

    /// Convert to protobuf for storage
    pub fn to_proto(&self) -> proto::GameConfig {
        proto::GameConfig {
            width: self.width as u32,
            height: self.height as u32,
            min_width: self.min_width as u32,
            min_height: self.min_height as u32,
            small_blind: self.small_blind,
            big_blind: self.big_blind,
            starting_chips: self.starting_chips,
            max_players: self.max_players as u32,
            min_players: 1,
            action_timeout: 0,
            ante: 0,
        }
    }

    /// Load configuration from a script file
    pub fn from_script<P: AsRef<Path>>(path: P) -> Self {
        let mut config = Self::default();

        if let Ok(mut interp) = Self::load_interpreter(path) {
            // Run the script to populate variables
            let _ = interp.run_until_sleep();

            // Extract values
            if let Some(Value::Int(w)) = interp.get("w") {
                config.width = (*w as u16).max(config.min_width);
            }
            if let Some(Value::Int(h)) = interp.get("h") {
                config.height = (*h as u16).max(config.min_height);
            }
            if let Some(Value::Int(mw)) = interp.get("min_w") {
                config.min_width = *mw as u16;
            }
            if let Some(Value::Int(mh)) = interp.get("min_h") {
                config.min_height = *mh as u16;
            }
            if let Some(Value::Int(sb)) = interp.get("small_blind") {
                config.small_blind = *sb as u32;
            }
            if let Some(Value::Int(bb)) = interp.get("big_blind") {
                config.big_blind = *bb as u32;
            }
            if let Some(Value::Int(chips)) = interp.get("starting_chips") {
                config.starting_chips = *chips as u32;
            }
            if let Some(Value::Int(max)) = interp.get("max_players") {
                config.max_players = *max as usize;
            }
        }

        config
    }

    fn load_interpreter<P: AsRef<Path>>(path: P) -> crate::engine::Result<Interpreter> {
        let mut interp = Interpreter::new();
        interp.load_file(path)?;
        Ok(interp)
    }
}

/// Player status in the game
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerStatus {
    Active,
    Folded,
    AllIn,
    Waiting,
    SittingOut,
}

/// A poker player
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub chips: u32,
    pub hole_cards: Option<[Card; 2]>,
    pub status: PlayerStatus,
    pub current_bet: u32,
    pub is_dealer: bool,
    pub is_local: bool,
}

impl Player {
    pub fn new(id: String, name: String, chips: u32, is_local: bool) -> Self {
        Self {
            id,
            name,
            chips,
            hole_cards: None,
            status: PlayerStatus::Waiting,
            current_bet: 0,
            is_dealer: false,
            is_local,
        }
    }

    /// Check if player can act
    pub fn can_act(&self) -> bool {
        matches!(self.status, PlayerStatus::Active)
    }

    /// Reset for new hand
    pub fn new_hand(&mut self) {
        self.hole_cards = None;
        self.current_bet = 0;
        if self.chips > 0 {
            self.status = PlayerStatus::Active;
        } else {
            self.status = PlayerStatus::SittingOut;
        }
    }
}

/// Betting round phases
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BettingRound {
    PreFlop,
    Flop,
    Turn,
    River,
    Showdown,
}

impl BettingRound {
    pub fn name(&self) -> &'static str {
        match self {
            BettingRound::PreFlop => "Pre-Flop",
            BettingRound::Flop => "Flop",
            BettingRound::Turn => "Turn",
            BettingRound::River => "River",
            BettingRound::Showdown => "Showdown",
        }
    }

    pub fn next(&self) -> Option<BettingRound> {
        match self {
            BettingRound::PreFlop => Some(BettingRound::Flop),
            BettingRound::Flop => Some(BettingRound::Turn),
            BettingRound::Turn => Some(BettingRound::River),
            BettingRound::River => Some(BettingRound::Showdown),
            BettingRound::Showdown => None,
        }
    }
}

/// Poker actions a player can take
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PokerAction {
    Fold,
    Check,
    Call,
    Raise(u32),
    AllIn,
}

impl PokerAction {
    pub fn name(&self) -> &'static str {
        match self {
            PokerAction::Fold => "Fold",
            PokerAction::Check => "Check",
            PokerAction::Call => "Call",
            PokerAction::Raise(_) => "Raise",
            PokerAction::AllIn => "All-In",
        }
    }
}

/// Selected action in the UI
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectedAction {
    Fold,
    Check,
    Call,
    Raise,
    AllIn,
}

impl SelectedAction {
    pub fn all() -> [SelectedAction; 5] {
        [
            SelectedAction::Fold,
            SelectedAction::Check,
            SelectedAction::Call,
            SelectedAction::Raise,
            SelectedAction::AllIn,
        ]
    }

    pub fn next(&self) -> SelectedAction {
        match self {
            SelectedAction::Fold => SelectedAction::Check,
            SelectedAction::Check => SelectedAction::Call,
            SelectedAction::Call => SelectedAction::Raise,
            SelectedAction::Raise => SelectedAction::AllIn,
            SelectedAction::AllIn => SelectedAction::Fold,
        }
    }

    pub fn prev(&self) -> SelectedAction {
        match self {
            SelectedAction::Fold => SelectedAction::AllIn,
            SelectedAction::Check => SelectedAction::Fold,
            SelectedAction::Call => SelectedAction::Check,
            SelectedAction::Raise => SelectedAction::Call,
            SelectedAction::AllIn => SelectedAction::Raise,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SelectedAction::Fold => "[F]old",
            SelectedAction::Check => "[C]heck",
            SelectedAction::Call => "Ca[l]l",
            SelectedAction::Raise => "[R]aise",
            SelectedAction::AllIn => "[A]ll-in",
        }
    }
}

/// Table state
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub pot: u32,
    pub current_bet: u32,
    pub small_blind: u32,
    pub big_blind: u32,
    pub community_cards: Vec<Card>,
    pub dealer_position: usize,
    pub action_position: usize,
}

impl Table {
    pub fn new(small_blind: u32, big_blind: u32) -> Self {
        Self {
            pot: 0,
            current_bet: 0,
            small_blind,
            big_blind,
            community_cards: Vec::new(),
            dealer_position: 0,
            action_position: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pot = 0;
        self.current_bet = 0;
        self.community_cards.clear();
    }

    /// Display community cards (showing only revealed ones)
    pub fn display_community(&self, round: BettingRound) -> String {
        let num_visible = match round {
            BettingRound::PreFlop => 0,
            BettingRound::Flop => 3,
            BettingRound::Turn => 4,
            BettingRound::River | BettingRound::Showdown => 5,
        };

        let mut display = String::new();
        for i in 0..5 {
            if i < num_visible && i < self.community_cards.len() {
                display.push_str(&at(&self.community_cards, i).display_bracketed());
            } else {
                display.push_str("[  ]");
            }
            if i < 4 {
                display.push(' ');
            }
        }
        display
    }
}

/// Main poker game state
pub struct PokerGame {
    universe: Universe,
    session: Session,
    pub config: GameConfig,
    pub players: Vec<Player>,
    pub table: Table,
    deck: Deck,
    pub round: BettingRound,
    pub room_id: String,
    pub local_player_index: usize,
    pub selected_action: SelectedAction,
    pub raise_amount: u32,
    pub game_over: bool,
    pub winner_message: Option<String>,
    pub chat_messages: Vec<ChatMessage>,
    // Storage-backed persistence
    storage: Option<Arc<Storage>>,
    hand_recorder: Option<HandRecorder>,
    pub user_id: String,
    pub buy_in_amount: u32,
}

/// Chat message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub sender: String,
    pub content: String,
    pub timestamp: i64,
}

impl PokerGame {
    /// Create a new poker game with default config
    pub fn new(universe: Universe) -> Self {
        Self::with_config(universe, GameConfig::default(), "You".to_string())
    }

    /// Create a new game with custom settings
    pub fn with_settings(
        universe: Universe,
        player_name: String,
        small_blind: u32,
        big_blind: u32,
        starting_chips: u32,
    ) -> Self {
        let config = GameConfig {
            small_blind,
            big_blind,
            starting_chips,
            ..Default::default()
        };
        Self::with_config(universe, config, player_name)
    }

    /// Create a new game from a config file
    pub fn from_config_file<P: AsRef<Path>>(
        universe: Universe,
        config_path: P,
        player_name: String,
    ) -> Self {
        let config = GameConfig::from_script(config_path);
        Self::with_config(universe, config, player_name)
    }

    /// Create a new game with explicit config
    pub fn with_config(universe: Universe, config: GameConfig, player_name: String) -> Self {
        let session = Session::new("poker".to_string());

        let local_player =
            Player::new(nanoid::nanoid!(8), player_name, config.starting_chips, true);

        let raise_amount = config.big_blind * 2;

        Self {
            universe,
            session,
            config: config.clone(),
            players: vec![local_player],
            table: Table::new(config.small_blind, config.big_blind),
            deck: Deck::new(),
            round: BettingRound::PreFlop,
            room_id: nanoid::nanoid!(6),
            local_player_index: 0,
            selected_action: SelectedAction::Check,
            raise_amount,
            game_over: false,
            winner_message: None,
            chat_messages: Vec::new(),
            storage: None,
            hand_recorder: None,
            user_id: String::new(),
            buy_in_amount: 0,
        }
    }

    /// Create a new game with storage backend for persistence
    pub fn with_config_and_storage(
        universe: Universe,
        config: GameConfig,
        player_name: String,
        storage: Arc<Storage>,
        user_id: String,
        buy_in: u32,
    ) -> Self {
        let session = Session::new("poker".to_string());

        let local_player =
            Player::new(nanoid::nanoid!(8), player_name, config.starting_chips, true);

        let raise_amount = config.big_blind * 2;

        Self {
            universe,
            session,
            config: config.clone(),
            players: vec![local_player],
            table: Table::new(config.small_blind, config.big_blind),
            deck: Deck::new(),
            round: BettingRound::PreFlop,
            room_id: nanoid::nanoid!(6),
            local_player_index: 0,
            selected_action: SelectedAction::Check,
            raise_amount,
            game_over: false,
            winner_message: None,
            chat_messages: Vec::new(),
            storage: Some(storage),
            hand_recorder: None,
            user_id,
            buy_in_amount: buy_in,
        }
    }

    /// Add a player to the game
    pub fn add_player(&mut self, name: String, chips: u32) -> usize {
        let id = nanoid::nanoid!(8);
        let player = Player::new(id, name, chips, false);
        self.players.push(player);
        self.players.len() - 1
    }

    /// Start a new hand
    pub fn start_hand(&mut self) {
        // Reset table
        self.table.reset();
        self.deck = Deck::new();
        self.round = BettingRound::PreFlop;
        self.game_over = false;
        self.winner_message = None;

        // Reset players
        for player in &mut self.players {
            player.new_hand();
        }

        // Move dealer button
        self.table.dealer_position = wrap_pos(self.table.dealer_position + 1, self.players.len());

        // Initialize hand recorder if storage is available
        if self.storage.is_some() {
            let player_data: Vec<(String, String, u32, bool, u32)> = self
                .players
                .iter()
                .enumerate()
                .map(|(i, p)| (p.id.clone(), p.name.clone(), p.chips, p.is_local, i as u32))
                .collect();
            self.hand_recorder = Some(HandRecorder::new(
                nanoid::nanoid!(10),
                &player_data,
                self.table.small_blind,
                self.table.big_blind,
                self.table.dealer_position as u32,
            ));
        }

        // Set dealer flag
        for (i, player) in self.players.iter_mut().enumerate() {
            player.is_dealer = i == self.table.dealer_position;
        }

        // Deal hole cards
        for player in &mut self.players {
            if player.status == PlayerStatus::Active {
                let card1 = match self.deck.deal() {
                    Some(c) => c,
                    None => std::process::abort(),
                };
                let card2 = match self.deck.deal() {
                    Some(c) => c,
                    None => std::process::abort(),
                };
                player.hole_cards = Some([card1, card2]);
            }
        }

        // Deal community cards (but don't reveal yet)
        self.deck.burn();
        for _ in 0..3 {
            if let Some(card) = self.deck.deal() {
                self.table.community_cards.push(card);
            }
        }
        self.deck.burn();
        if let Some(card) = self.deck.deal() {
            self.table.community_cards.push(card);
        }
        self.deck.burn();
        if let Some(card) = self.deck.deal() {
            self.table.community_cards.push(card);
        }

        // Collect blinds
        self.collect_blinds();

        // Set action to player after big blind
        let num_players = self.players.len();
        if num_players > 2 {
            self.table.action_position = wrap_pos(self.table.dealer_position + 3, num_players);
        } else {
            self.table.action_position = self.table.dealer_position;
        }
    }

    /// Collect blinds from players
    fn collect_blinds(&mut self) {
        let num_players = self.players.len();
        if num_players < 2 {
            return;
        }

        // Small blind
        let sb_pos = wrap_pos(self.table.dealer_position + 1, num_players);
        let sb_amount = self.table.small_blind.min(at(&self.players, sb_pos).chips);
        at_mut(&mut self.players, sb_pos).chips -= sb_amount;
        at_mut(&mut self.players, sb_pos).current_bet = sb_amount;
        self.table.pot += sb_amount;

        // Big blind
        let bb_pos = wrap_pos(self.table.dealer_position + 2, num_players);
        let bb_amount = self.table.big_blind.min(at(&self.players, bb_pos).chips);
        at_mut(&mut self.players, bb_pos).chips -= bb_amount;
        at_mut(&mut self.players, bb_pos).current_bet = bb_amount;
        self.table.pot += bb_amount;
        self.table.current_bet = bb_amount;
    }

    /// Execute a poker action
    pub fn execute_action(&mut self, action: PokerAction) {
        let player_idx = self.table.action_position;

        // Record action for hand history
        if let Some(ref mut recorder) = self.hand_recorder {
            let pid = at(&self.players, player_idx).id.clone();
            let (action_type, amount) = match &action {
                PokerAction::Fold => (proto::ActionType::ActionFold, 0),
                PokerAction::Check => (proto::ActionType::ActionCheck, 0),
                PokerAction::Call => (
                    proto::ActionType::ActionCall,
                    self.table.current_bet - at(&self.players, player_idx).current_bet,
                ),
                PokerAction::Raise(amt) => (proto::ActionType::ActionRaise, *amt),
                PokerAction::AllIn => (
                    proto::ActionType::ActionAllIn,
                    at(&self.players, player_idx).chips,
                ),
            };
            recorder.record_action(&pid, action_type, amount);
        }

        let player = at_mut(&mut self.players, player_idx);

        match action {
            PokerAction::Fold => {
                player.status = PlayerStatus::Folded;
            }
            PokerAction::Check => {
                // Check is valid if current_bet == player's bet
            }
            PokerAction::Call => {
                let call_amount = self.table.current_bet - player.current_bet;
                let actual = call_amount.min(player.chips);
                player.chips -= actual;
                player.current_bet += actual;
                self.table.pot += actual;
                if player.chips == 0 {
                    player.status = PlayerStatus::AllIn;
                }
            }
            PokerAction::Raise(amount) => {
                let total_bet = self.table.current_bet + amount;
                let to_add = total_bet - player.current_bet;
                let actual = to_add.min(player.chips);
                player.chips -= actual;
                player.current_bet += actual;
                self.table.pot += actual;
                self.table.current_bet = player.current_bet;
                if player.chips == 0 {
                    player.status = PlayerStatus::AllIn;
                }
            }
            PokerAction::AllIn => {
                let amount = player.chips;
                player.current_bet += amount;
                self.table.pot += amount;
                player.chips = 0;
                player.status = PlayerStatus::AllIn;
                if player.current_bet > self.table.current_bet {
                    self.table.current_bet = player.current_bet;
                }
            }
        }

        // Check if hand is over
        if self.check_hand_over() {
            self.determine_winner();
            return;
        }

        // Move to next player or next round
        self.advance_action();
    }

    /// Check if only one player remains
    fn check_hand_over(&self) -> bool {
        let active = self
            .players
            .iter()
            .filter(|p| matches!(p.status, PlayerStatus::Active | PlayerStatus::AllIn))
            .count();
        active <= 1
    }

    /// Advance to next player or round
    fn advance_action(&mut self) {
        let num_players = self.players.len();
        let mut next = wrap_pos(self.table.action_position + 1, num_players);
        let starting = next;

        // Find next active player
        loop {
            if at(&self.players, next).can_act() {
                break;
            }
            next = wrap_pos(next + 1, num_players);
            if next == starting {
                // No more active players or round complete
                self.advance_round();
                return;
            }
        }

        // Check if betting round is complete
        let all_matched = self
            .players
            .iter()
            .all(|p| !p.can_act() || p.current_bet == self.table.current_bet);

        if all_matched && next == self.first_to_act() {
            self.advance_round();
        } else {
            self.table.action_position = next;
        }
    }

    /// Get first player to act in current round
    fn first_to_act(&self) -> usize {
        let num_players = self.players.len();
        if self.round == BettingRound::PreFlop {
            wrap_pos(self.table.dealer_position + 3, num_players)
        } else {
            wrap_pos(self.table.dealer_position + 1, num_players)
        }
    }

    /// Advance to next betting round
    fn advance_round(&mut self) {
        // Record round advancement for hand history
        let round_num = match self.round {
            BettingRound::PreFlop => 0,
            BettingRound::Flop => 1,
            BettingRound::Turn => 2,
            BettingRound::River => 3,
            BettingRound::Showdown => 4,
        };
        if let Some(ref mut recorder) = self.hand_recorder {
            recorder.advance_round(round_num);
        }

        // Reset current bets
        for player in &mut self.players {
            player.current_bet = 0;
        }
        self.table.current_bet = 0;

        // Move to next round
        if let Some(next_round) = self.round.next() {
            self.round = next_round;

            if self.round == BettingRound::Showdown {
                self.determine_winner();
            } else {
                // Set action to first active player after dealer
                self.table.action_position = self.first_to_act();
            }
        } else {
            self.determine_winner();
        }
    }

    /// Determine the winner(s) and award pot
    fn determine_winner(&mut self) {
        self.game_over = true;
        self.round = BettingRound::Showdown;

        let active_players: Vec<(usize, EvaluatedHand)> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.status, PlayerStatus::Active | PlayerStatus::AllIn))
            .filter_map(|(i, p)| {
                if let Some(hole) = p.hole_cards {
                    let mut all_cards = self.table.community_cards.clone();
                    all_cards.push(*at(&hole, 0));
                    all_cards.push(*at(&hole, 1));
                    Some((i, evaluate_hand(&all_cards)))
                } else {
                    None
                }
            })
            .collect();

        let mut winner_id = String::new();
        let mut winning_rank: u32 = 0;
        let mut winning_desc = String::new();
        let pot = self.table.pot;

        if active_players.is_empty() {
            // Everyone folded - award to last player standing
            for player in self.players.iter_mut() {
                if matches!(player.status, PlayerStatus::Active | PlayerStatus::AllIn) {
                    player.chips += pot;
                    winner_id = player.id.clone();
                    self.winner_message = Some(format!("{} wins ${}!", player.name, pot));
                    break;
                }
            }
        } else if active_players.len() == 1 {
            let (idx, _) = at(&active_players, 0);
            at_mut(&mut self.players, *idx).chips += pot;
            winner_id = at(&self.players, *idx).id.clone();
            self.winner_message = Some(format!("{} wins ${}!", at(&self.players, *idx).name, pot));
        } else {
            // Find best hand
            let (idx, best_hand) = match active_players.iter().max_by(|(_, h1), (_, h2)| h1.cmp(h2))
            {
                Some(w) => w,
                None => std::process::abort(),
            };

            at_mut(&mut self.players, *idx).chips += pot;
            winner_id = at(&self.players, *idx).id.clone();
            winning_rank = best_hand.rank_value();
            winning_desc = best_hand.description();
            self.winner_message = Some(format!(
                "{} wins ${} with {}!",
                at(&self.players, *idx).name,
                pot,
                best_hand.description()
            ));
        }

        // Save hand record and update user stats
        self.save_hand_record(&winner_id, pot, winning_rank, &winning_desc);
    }

    /// Persist hand record and update user stats
    fn save_hand_record(
        &mut self,
        winner_id: &str,
        pot: u32,
        winning_rank: u32,
        winning_desc: &str,
    ) {
        let storage = match self.storage.as_ref() {
            Some(s) => s,
            None => return,
        };

        // Build player chip snapshots
        let chip_data: Vec<(String, u32)> = self
            .players
            .iter()
            .map(|p| (p.id.clone(), p.chips))
            .collect();

        // Encode community cards
        let cc: Vec<(u8, u8)> = self
            .table
            .community_cards
            .iter()
            .map(|c| (c.rank as u8, c.suit as u8))
            .collect();

        // Finish and save the hand record
        if let Some(recorder) = self.hand_recorder.take() {
            let _ = recorder.finish(
                storage,
                &chip_data,
                &cc,
                winner_id,
                pot,
                winning_rank,
                winning_desc,
            );
        }

        // Update user stats
        let local_won = winner_id == at(&self.players, self.local_player_index).id;
        if let Ok(Some(mut user)) = storage.get_user() {
            let _ = storage::user::update_stats_after_hand(
                storage,
                &mut user,
                local_won,
                pot,
                winning_rank,
            );
        }
    }

    /// Check if it's the local player's turn
    pub fn is_local_turn(&self) -> bool {
        !self.game_over
            && self.table.action_position == self.local_player_index
            && at(&self.players, self.local_player_index).can_act()
    }

    /// Get available actions for current player
    pub fn available_actions(&self) -> Vec<SelectedAction> {
        let player = at(&self.players, self.table.action_position);
        let mut actions = vec![SelectedAction::Fold];

        if player.current_bet == self.table.current_bet {
            actions.push(SelectedAction::Check);
        } else {
            actions.push(SelectedAction::Call);
        }

        if player.chips > self.table.current_bet - player.current_bet {
            actions.push(SelectedAction::Raise);
        }

        actions.push(SelectedAction::AllIn);
        actions
    }

    /// Handle input and return game action
    pub fn tick(&mut self, input: Input) -> GameAction {
        match input {
            Input::Quit => return GameAction::Quit,
            Input::Action => {
                if self.game_over {
                    // Start new hand on Enter when game is over
                    self.start_hand();
                } else if self.is_local_turn() {
                    // Execute selected action
                    let action = match self.selected_action {
                        SelectedAction::Fold => PokerAction::Fold,
                        SelectedAction::Check => PokerAction::Check,
                        SelectedAction::Call => PokerAction::Call,
                        SelectedAction::Raise => PokerAction::Raise(self.raise_amount),
                        SelectedAction::AllIn => PokerAction::AllIn,
                    };
                    self.execute_action(action);
                }
            }
            Input::Move(Direction::Left) => {
                if self.is_local_turn() {
                    self.selected_action = self.selected_action.prev();
                }
            }
            Input::Move(Direction::Right) => {
                if self.is_local_turn() {
                    self.selected_action = self.selected_action.next();
                }
            }
            Input::Move(Direction::Up) => {
                if self.is_local_turn() && self.selected_action == SelectedAction::Raise {
                    self.raise_amount = (self.raise_amount + self.table.big_blind)
                        .min(at(&self.players, self.local_player_index).chips);
                }
            }
            Input::Move(Direction::Down) => {
                if self.is_local_turn() && self.selected_action == SelectedAction::Raise {
                    self.raise_amount = self
                        .raise_amount
                        .saturating_sub(self.table.big_blind)
                        .max(self.table.big_blind);
                }
            }
            // Direct poker action keys
            Input::Fold => {
                if self.is_local_turn() {
                    self.execute_action(PokerAction::Fold);
                }
            }
            Input::Check => {
                if self.is_local_turn() {
                    // Check is only valid if current bet matches
                    let player = at(&self.players, self.local_player_index);
                    if player.current_bet == self.table.current_bet {
                        self.execute_action(PokerAction::Check);
                    }
                }
            }
            Input::Call => {
                if self.is_local_turn() {
                    self.execute_action(PokerAction::Call);
                }
            }
            Input::Raise => {
                if self.is_local_turn() {
                    self.selected_action = SelectedAction::Raise;
                }
            }
            Input::AllIn => {
                if self.is_local_turn() {
                    self.execute_action(PokerAction::AllIn);
                }
            }
            // Number input for raise amount
            Input::Number(n) => {
                if self.is_local_turn() && self.selected_action == SelectedAction::Raise {
                    // Append digit to raise amount
                    let new_amount = self
                        .raise_amount
                        .saturating_mul(10)
                        .saturating_add(n as u32);
                    self.raise_amount =
                        new_amount.min(at(&self.players, self.local_player_index).chips);
                }
            }
            Input::Backspace => {
                if self.is_local_turn() && self.selected_action == SelectedAction::Raise {
                    // Remove last digit from raise amount
                    self.raise_amount /= 10;
                    if self.raise_amount < self.table.big_blind {
                        self.raise_amount = self.table.big_blind;
                    }
                }
            }
            // Chat input (to be implemented with chat system)
            Input::Char(_c) => {
                // TODO: Add chat functionality
            }
            Input::None => {}
        }

        GameAction::Continue
    }

    /// Render the game state to a Map
    pub fn render(&self) -> Map {
        self.universe.map.clone()
    }

    pub fn get_session(&self) -> &Session {
        &self.session
    }

    pub fn get_universe(&self) -> &Universe {
        &self.universe
    }
}

/// Safe position wrapping — avoids panic branch that embeds file paths in binary
#[inline(always)]
fn wrap_pos(value: usize, count: usize) -> usize {
    value.checked_rem(count).unwrap_or(0)
}

/// Create default poker universe
pub fn create_poker_universe() -> Universe {
    let mut universe = Universe::new("poker".to_string(), 60, 24);
    universe.objective = crate::engine::Objective {
        description: "Win chips by making the best hand or bluffing your opponents!".to_string(),
    };
    universe.rules = crate::engine::Rules {
        allow_movement: false,
        allow_teleport: true,
        max_width: 80,
        max_height: 30,
    };
    universe.session_config = crate::engine::SessionConfig {
        timeout_seconds: 3600,
        save_history: true,
    };
    universe
}
