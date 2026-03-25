use crate::engine::error::{GameError, Result};
use crate::storage::database::Storage;
use crate::storage::schema;
use chrono::Utc;
use obfstr::obfstr;

/// Wallet manager for chip persistence
pub struct WalletManager<'a> {
    storage: &'a Storage,
}

impl<'a> WalletManager<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    /// Get current balance for the user
    pub fn get_balance(&self) -> Result<u64> {
        match self.storage.get_wallet()? {
            Some(wallet) => Ok(wallet.balance),
            None => Ok(0),
        }
    }

    /// Debit chips from wallet (buy-in)
    pub fn buy_in(&self, _user_id: &str, amount: u32) -> Result<u32> {
        let mut wallet = match self.storage.get_wallet()? {
            Some(w) => w,
            None => return Err(GameError::Storage("No wallet found".into())),
        };

        let actual_amount = (amount as u64).min(wallet.balance) as u32;
        if actual_amount == 0 {
            return Err(GameError::Storage("Insufficient balance".into()));
        }

        wallet.balance -= actual_amount as u64;
        wallet.last_updated = Utc::now().timestamp();
        self.storage.save_wallet(&wallet)?;

        // Record transaction
        let tx = schema::Transaction {
            tx_id: nanoid::nanoid!(10),
            tx_type: schema::TransactionType::TxBlindPost as i32,
            amount: -(actual_amount as i64),
            balance_after: wallet.balance,
            timestamp: Utc::now().timestamp(),
            hand_id: String::new(),
            description: obfstr!("Game buy-in").to_string(),
        };
        self.storage.save_transaction(&tx)?;

        Ok(actual_amount)
    }

    /// Credit chips back to wallet (cash-out)
    pub fn cash_out(
        &self,
        _user_id: &str,
        chips_remaining: u32,
        original_buy_in: u32,
    ) -> Result<()> {
        let mut wallet = match self.storage.get_wallet()? {
            Some(w) => w,
            None => return Err(GameError::Storage("No wallet found".into())),
        };

        wallet.balance += chips_remaining as u64;
        wallet.last_updated = Utc::now().timestamp();
        self.storage.save_wallet(&wallet)?;

        // Record the net result transaction
        let net = chips_remaining as i64 - original_buy_in as i64;
        let tx_type = if net >= 0 {
            schema::TransactionType::TxHandWin as i32
        } else {
            schema::TransactionType::TxHandLoss as i32
        };

        let tx = schema::Transaction {
            tx_id: nanoid::nanoid!(10),
            tx_type,
            amount: chips_remaining as i64,
            balance_after: wallet.balance,
            timestamp: Utc::now().timestamp(),
            hand_id: String::new(),
            description: obfstr!("Game cash-out").to_string(),
        };
        self.storage.save_transaction(&tx)?;

        Ok(())
    }

    /// Credit chips for a hand win (during game)
    pub fn record_hand_result(&self, hand_id: &str, net_amount: i64) -> Result<()> {
        let wallet = match self.storage.get_wallet()? {
            Some(w) => w,
            None => return Err(GameError::Storage("No wallet found".into())),
        };

        // Net amount is already applied to in-game chips;
        // we just record the transaction for history
        let tx_type = if net_amount >= 0 {
            schema::TransactionType::TxHandWin as i32
        } else {
            schema::TransactionType::TxHandLoss as i32
        };

        let tx = schema::Transaction {
            tx_id: nanoid::nanoid!(10),
            tx_type,
            amount: net_amount,
            balance_after: wallet.balance,
            timestamp: Utc::now().timestamp(),
            hand_id: hand_id.to_string(),
            description: String::new(),
        };
        self.storage.save_transaction(&tx)?;

        Ok(())
    }
}
