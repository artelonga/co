use rand::seq::SliceRandom;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Card suits
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Suit {
    Hearts,
    Diamonds,
    Clubs,
    Spades,
}

impl Suit {
    /// Get Unicode symbol for the suit
    pub fn symbol(&self) -> &'static str {
        match self {
            Suit::Hearts => "\u{2665}",   // ♥
            Suit::Diamonds => "\u{2666}", // ♦
            Suit::Clubs => "\u{2663}",    // ♣
            Suit::Spades => "\u{2660}",   // ♠
        }
    }

    /// Check if suit is red (hearts/diamonds)
    pub fn is_red(&self) -> bool {
        matches!(self, Suit::Hearts | Suit::Diamonds)
    }

    /// All suits in order
    pub fn all() -> [Suit; 4] {
        [Suit::Hearts, Suit::Diamonds, Suit::Clubs, Suit::Spades]
    }
}

impl fmt::Display for Suit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

/// Card ranks (2-10, J, Q, K, A)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rank {
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Nine = 9,
    Ten = 10,
    Jack = 11,
    Queen = 12,
    King = 13,
    Ace = 14,
}

impl Rank {
    /// Get display character for the rank
    pub fn symbol(&self) -> &'static str {
        match self {
            Rank::Two => "2",
            Rank::Three => "3",
            Rank::Four => "4",
            Rank::Five => "5",
            Rank::Six => "6",
            Rank::Seven => "7",
            Rank::Eight => "8",
            Rank::Nine => "9",
            Rank::Ten => "10",
            Rank::Jack => "J",
            Rank::Queen => "Q",
            Rank::King => "K",
            Rank::Ace => "A",
        }
    }

    /// Numeric value (2-14, Ace high)
    pub fn value(&self) -> u8 {
        *self as u8
    }

    /// All ranks in order (2 to Ace)
    pub fn all() -> [Rank; 13] {
        [
            Rank::Two,
            Rank::Three,
            Rank::Four,
            Rank::Five,
            Rank::Six,
            Rank::Seven,
            Rank::Eight,
            Rank::Nine,
            Rank::Ten,
            Rank::Jack,
            Rank::Queen,
            Rank::King,
            Rank::Ace,
        ]
    }
}

impl fmt::Display for Rank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

/// A playing card
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }

    /// Format card for display (e.g., "7♥", "K♠")
    pub fn display(&self) -> String {
        format!("{}{}", self.rank.symbol(), self.suit.symbol())
    }

    /// Format card with ANSI colors (red for hearts/diamonds)
    pub fn display_colored(&self) -> String {
        if self.suit.is_red() {
            format!(
                "\x1b[31m{}{}\x1b[0m",
                self.rank.symbol(),
                self.suit.symbol()
            )
        } else {
            format!("{}{}", self.rank.symbol(), self.suit.symbol())
        }
    }

    /// Display as hidden card
    pub fn display_hidden() -> &'static str {
        "[??]"
    }

    /// Display in bracket format [7♥]
    pub fn display_bracketed(&self) -> String {
        if self.suit.is_red() {
            format!(
                "[\x1b[31m{}{}\x1b[0m]",
                self.rank.symbol(),
                self.suit.symbol()
            )
        } else {
            format!("[{}{}]", self.rank.symbol(), self.suit.symbol())
        }
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// A standard 52-card deck
#[derive(Clone, Debug)]
pub struct Deck {
    cards: Vec<Card>,
    dealt: usize,
}

impl Deck {
    /// Create a new shuffled deck
    pub fn new() -> Self {
        let mut deck = Self::new_ordered();
        deck.shuffle();
        deck
    }

    /// Create a new deck in order (not shuffled)
    pub fn new_ordered() -> Self {
        let mut cards = Vec::with_capacity(52);
        for suit in Suit::all() {
            for rank in Rank::all() {
                cards.push(Card::new(rank, suit));
            }
        }
        Self { cards, dealt: 0 }
    }

    /// Shuffle the deck and reset dealt count
    pub fn shuffle(&mut self) {
        let mut rng = thread_rng();
        self.cards.shuffle(&mut rng);
        self.dealt = 0;
    }

    /// Deal one card from the deck
    pub fn deal(&mut self) -> Option<Card> {
        match self.cards.get(self.dealt) {
            Some(&card) => {
                self.dealt += 1;
                Some(card)
            }
            None => None,
        }
    }

    /// Deal multiple cards
    pub fn deal_n(&mut self, n: usize) -> Vec<Card> {
        let mut cards = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(card) = self.deal() {
                cards.push(card);
            }
        }
        cards
    }

    /// Number of cards remaining
    pub fn remaining(&self) -> usize {
        self.cards.len() - self.dealt
    }

    /// Reset the deck (reshuffle all cards)
    pub fn reset(&mut self) {
        self.shuffle();
    }

    /// Burn a card (deal without keeping)
    pub fn burn(&mut self) {
        self.deal();
    }
}

impl Default for Deck {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deck_has_52_cards() {
        let deck = Deck::new_ordered();
        assert_eq!(deck.remaining(), 52);
    }

    #[test]
    fn test_deal_reduces_remaining() {
        let mut deck = Deck::new();
        deck.deal();
        assert_eq!(deck.remaining(), 51);
    }

    #[test]
    fn test_card_display() {
        let card = Card::new(Rank::Ace, Suit::Spades);
        assert_eq!(card.display(), "A\u{2660}");
    }

    #[test]
    fn test_rank_ordering() {
        assert!(Rank::Ace > Rank::King);
        assert!(Rank::King > Rank::Queen);
        assert!(Rank::Two < Rank::Three);
    }

    #[test]
    fn test_suit_colors() {
        assert!(Suit::Hearts.is_red());
        assert!(Suit::Diamonds.is_red());
        assert!(!Suit::Clubs.is_red());
        assert!(!Suit::Spades.is_red());
    }
}
