use game_core::storage::database::Storage;
use game_core::Result;

/// Show hand history
pub fn run_history(storage: &Storage, limit: usize) -> Result<()> {
    let hands = game_core::storage::history::list_recent_hands(storage, limit)?;

    if hands.is_empty() {
        println!("No hands played yet.");
        return Ok(());
    }

    println!("=== Hand History ({} hands) ===", hands.len());
    println!();

    for hand in &hands {
        let winner_name = hand
            .players
            .iter()
            .find(|p| p.player_id == hand.winner_player_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown");

        let desc = if hand.winning_hand_description.is_empty() {
            "fold".to_string()
        } else {
            hand.winning_hand_description.clone()
        };

        println!(
            "Hand {} | Pot: ${} | Winner: {} ({})",
            hand.hand_id, hand.pot_amount, winner_name, desc,
        );

        // Show player results
        for player in &hand.players {
            let net = player.chips_end as i64 - player.chips_start as i64;
            let sign = if net >= 0 { "+" } else { "" };
            println!(
                "  {} {} ${} -> ${} ({}${})",
                if player.is_local { ">" } else { " " },
                player.name,
                player.chips_start,
                player.chips_end,
                sign,
                net,
            );
        }
        println!();
    }

    Ok(())
}
