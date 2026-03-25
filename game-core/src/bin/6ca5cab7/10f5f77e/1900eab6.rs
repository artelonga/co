use game_core::storage::database::Storage;
use game_core::Result;

/// Show user profile and stats
pub fn run_profile(storage: &Storage) -> Result<()> {
    let user = match storage.get_user()? {
        Some(u) => u,
        None => {
            println!("No profile found. Play a game first!");
            return Ok(());
        }
    };

    println!("=== Player Profile ===");
    println!("Name:    {}", user.display_name);
    println!("ID:      {}", user.user_id);

    if let Some(stats) = &user.stats {
        println!();
        println!("=== Statistics ===");
        println!("Sessions:    {}", stats.sessions_played);
        println!("Hands:       {}", stats.hands_played);
        println!("Wins:        {}", stats.hands_won);
        if stats.hands_played > 0 {
            let win_rate = (stats.hands_won as f64 / stats.hands_played as f64) * 100.0;
            println!("Win rate:    {:.1}%", win_rate);
        }
        println!("Chips won:   ${}", stats.total_chips_won);
        println!("Chips lost:  ${}", stats.total_chips_lost);
        println!("Biggest pot: ${}", stats.biggest_pot_won);
        if stats.best_hand_rank > 0 {
            let rank_name = match stats.best_hand_rank {
                0 => "High Card",
                1 => "One Pair",
                2 => "Two Pair",
                3 => "Three of a Kind",
                4 => "Straight",
                5 => "Flush",
                6 => "Full House",
                7 => "Four of a Kind",
                8 => "Straight Flush",
                9 => "Royal Flush",
                _ => "Unknown",
            };
            println!("Best hand:   {}", rank_name);
        }
    }

    Ok(())
}
