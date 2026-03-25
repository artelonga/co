use crate::engine::error::Result;
use crate::storage::database::Storage;
use crate::storage::schema;
use chrono::Utc;

/// Records actions during a hand for later persistence
pub struct HandRecorder {
    record: schema::HandRecord,
    current_round_actions: Vec<schema::PlayerAction>,
}

impl HandRecorder {
    /// Start recording a new hand
    pub fn new(
        hand_id: String,
        players: &[(String, String, u32, bool, u32)], // (id, name, chips, is_local, seat)
        small_blind: u32,
        big_blind: u32,
        dealer_position: u32,
    ) -> Self {
        let hand_players: Vec<schema::HandPlayer> = players
            .iter()
            .map(|(id, name, chips, is_local, seat)| schema::HandPlayer {
                player_id: id.clone(),
                name: name.clone(),
                chips_start: *chips,
                chips_end: 0,
                seat_position: *seat,
                is_local: *is_local,
                hole_cards: Vec::new(),
            })
            .collect();

        let record = schema::HandRecord {
            hand_id,
            started_at: Utc::now().timestamp(),
            ended_at: 0,
            small_blind,
            big_blind,
            dealer_position,
            players: hand_players,
            community_cards: Vec::new(),
            rounds: Vec::new(),
            winner_player_id: String::new(),
            pot_amount: 0,
            winning_hand_rank: 0,
            winning_hand_description: String::new(),
        };

        Self {
            record,
            current_round_actions: Vec::new(),
        }
    }

    /// Record a player action in the current betting round
    pub fn record_action(&mut self, player_id: &str, action: schema::ActionType, amount: u32) {
        self.current_round_actions.push(schema::PlayerAction {
            player_id: player_id.to_string(),
            action: action as i32,
            amount,
            timestamp: Utc::now().timestamp(),
        });
    }

    /// Advance to the next betting round, saving current actions
    pub fn advance_round(&mut self, round_num: u32) {
        if !self.current_round_actions.is_empty() {
            let round_record = schema::BettingRoundRecord {
                round: round_num,
                actions: std::mem::take(&mut self.current_round_actions),
            };
            self.record.rounds.push(round_record);
        }
    }

    /// Encode a card as rank_value * 4 + suit_index
    pub fn encode_card(rank: u8, suit: u8) -> u32 {
        (rank as u32) * 4 + (suit as u32)
    }

    /// Finish recording and save to storage
    #[allow(clippy::too_many_arguments)]
    pub fn finish(
        mut self,
        storage: &Storage,
        player_chips_end: &[(String, u32)],
        community_cards: &[(u8, u8)], // (rank, suit) pairs
        winner_id: &str,
        pot: u32,
        winning_rank: u32,
        winning_desc: &str,
    ) -> Result<String> {
        self.record.ended_at = Utc::now().timestamp();
        self.record.winner_player_id = winner_id.to_string();
        self.record.pot_amount = pot;
        self.record.winning_hand_rank = winning_rank;
        self.record.winning_hand_description = winning_desc.to_string();

        // Flush any remaining actions for the final round
        if !self.current_round_actions.is_empty() {
            let round_record = schema::BettingRoundRecord {
                round: self.record.rounds.len() as u32,
                actions: std::mem::take(&mut self.current_round_actions),
            };
            self.record.rounds.push(round_record);
        }

        // Update player chip counts at end
        for (pid, chips) in player_chips_end {
            for player in &mut self.record.players {
                if player.player_id == *pid {
                    player.chips_end = *chips;
                }
            }
        }

        // Encode community cards
        self.record.community_cards = community_cards
            .iter()
            .map(|(rank, suit)| Self::encode_card(*rank, *suit))
            .collect();

        let hand_id = self.record.hand_id.clone();
        storage.save_hand(&self.record)?;
        Ok(hand_id)
    }

    /// Get the hand ID
    pub fn hand_id(&self) -> &str {
        &self.record.hand_id
    }
}

/// List recent hands from storage
pub fn list_recent_hands(storage: &Storage, limit: usize) -> Result<Vec<schema::HandRecord>> {
    let ids = storage.list_hand_ids()?;
    let mut hands = Vec::new();

    // Take the last N hand IDs (most recent)
    let start = if ids.len() > limit {
        ids.len() - limit
    } else {
        0
    };

    for id in ids.get(start..).unwrap_or(&[]) {
        if let Some(hand) = storage.get_hand(id)? {
            hands.push(hand);
        }
    }

    Ok(hands)
}
