use game_core::storage::database::Storage;
use game_core::Result;

/// Show wallet balance
pub fn run_balance(storage: &Storage) -> Result<()> {
    let user = match storage.get_user()? {
        Some(u) => u,
        None => {
            println!("No profile found. Play a game first!");
            return Ok(());
        }
    };

    let wallet = game_core::storage::WalletManager::new(storage);
    let balance = wallet.get_balance()?;

    println!("{}: ${}", user.display_name, balance);

    // Show recent transactions
    let tx_ids = storage.list_transaction_ids()?;
    if !tx_ids.is_empty() {
        println!();
        println!("Recent transactions:");
        let start = if tx_ids.len() > 10 {
            tx_ids.len() - 10
        } else {
            0
        };
        for id in tx_ids.get(start..).unwrap_or(&[]) {
            if let Ok(Some(tx)) = storage.get_transaction(id) {
                let sign = if tx.amount >= 0 { "+" } else { "" };
                let desc = if tx.description.is_empty() {
                    match tx.tx_type {
                        1 => "Initial grant",
                        2 => "Hand win",
                        3 => "Hand loss",
                        4 => "Blind post",
                        5 => "Rebuy",
                        6 => "Daily bonus",
                        _ => "Unknown",
                    }
                } else {
                    &tx.description
                };
                println!(
                    "  {}${} - {} (bal: ${})",
                    sign, tx.amount, desc, tx.balance_after
                );
            }
        }
    }

    Ok(())
}
