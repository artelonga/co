use super::deck::Card;
use crate::engine::error::{at, at_mut};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

/// Poker hand rankings (lowest to highest)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum HandRank {
    HighCard = 0,
    OnePair = 1,
    TwoPair = 2,
    ThreeOfAKind = 3,
    Straight = 4,
    Flush = 5,
    FullHouse = 6,
    FourOfAKind = 7,
    StraightFlush = 8,
    RoyalFlush = 9,
}

impl HandRank {
    pub fn name(&self) -> &'static str {
        match self {
            HandRank::HighCard => "High Card",
            HandRank::OnePair => "One Pair",
            HandRank::TwoPair => "Two Pair",
            HandRank::ThreeOfAKind => "Three of a Kind",
            HandRank::Straight => "Straight",
            HandRank::Flush => "Flush",
            HandRank::FullHouse => "Full House",
            HandRank::FourOfAKind => "Four of a Kind",
            HandRank::StraightFlush => "Straight Flush",
            HandRank::RoyalFlush => "Royal Flush",
        }
    }
}

/// Evaluated hand with ranking and kickers for comparison
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluatedHand {
    pub rank: HandRank,
    /// Cards used in the hand ranking (for display)
    pub cards: Vec<Card>,
    /// Kicker values for tie-breaking (highest first)
    pub kickers: Vec<u8>,
}

impl EvaluatedHand {
    /// Numeric rank value (0-9) for storage
    pub fn rank_value(&self) -> u32 {
        self.rank as u32
    }

    pub fn description(&self) -> String {
        match self.rank {
            HandRank::RoyalFlush => "Royal Flush".to_string(),
            HandRank::StraightFlush => format!("Straight Flush, {} high", self.high_card_name()),
            HandRank::FourOfAKind => format!("Four of a Kind, {}s", self.primary_rank_name()),
            HandRank::FullHouse => format!(
                "Full House, {}s full of {}s",
                self.primary_rank_name(),
                self.secondary_rank_name()
            ),
            HandRank::Flush => format!("Flush, {} high", self.high_card_name()),
            HandRank::Straight => format!("Straight, {} high", self.high_card_name()),
            HandRank::ThreeOfAKind => format!("Three of a Kind, {}s", self.primary_rank_name()),
            HandRank::TwoPair => format!(
                "Two Pair, {}s and {}s",
                self.primary_rank_name(),
                self.secondary_rank_name()
            ),
            HandRank::OnePair => format!("Pair of {}s", self.primary_rank_name()),
            HandRank::HighCard => format!("{} high", self.high_card_name()),
        }
    }

    fn high_card_name(&self) -> &'static str {
        rank_name(self.kickers.first().copied().unwrap_or(2))
    }

    fn primary_rank_name(&self) -> &'static str {
        rank_name(self.kickers.first().copied().unwrap_or(2))
    }

    fn secondary_rank_name(&self) -> &'static str {
        rank_name(self.kickers.get(1).copied().unwrap_or(2))
    }
}

impl Ord for EvaluatedHand {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.rank.cmp(&other.rank) {
            Ordering::Equal => self.kickers.cmp(&other.kickers),
            ord => ord,
        }
    }
}

impl PartialOrd for EvaluatedHand {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn rank_name(value: u8) -> &'static str {
    match value {
        2 => "Two",
        3 => "Three",
        4 => "Four",
        5 => "Five",
        6 => "Six",
        7 => "Seven",
        8 => "Eight",
        9 => "Nine",
        10 => "Ten",
        11 => "Jack",
        12 => "Queen",
        13 => "King",
        14 => "Ace",
        _ => "Unknown",
    }
}

/// Evaluate the best 5-card hand from up to 7 cards
pub fn evaluate_hand(cards: &[Card]) -> EvaluatedHand {
    if cards.len() < 5 {
        // Not enough cards - return high card
        let mut sorted: Vec<_> = cards.iter().map(|c| c.rank.value()).collect();
        sorted.sort_by(|a, b| b.cmp(a));
        return EvaluatedHand {
            rank: HandRank::HighCard,
            cards: cards.to_vec(),
            kickers: sorted,
        };
    }

    // Generate all 5-card combinations
    let combinations = combinations_5(cards);
    let mut best: Option<EvaluatedHand> = None;

    for combo in combinations {
        let hand = evaluate_5_cards(&combo);
        if best.as_ref().is_none_or(|b| hand > *b) {
            best = Some(hand);
        }
    }

    match best {
        Some(h) => h,
        None => std::process::abort(),
    }
}

/// Evaluate exactly 5 cards
fn evaluate_5_cards(cards: &[Card]) -> EvaluatedHand {
    let mut sorted_cards = cards.to_vec();
    sorted_cards.sort_by_key(|b| std::cmp::Reverse(b.rank));

    let is_flush = check_flush(&sorted_cards);
    let straight_high = check_straight(&sorted_cards);

    // Count ranks
    let mut rank_counts: HashMap<u8, usize> = HashMap::new();
    for card in &sorted_cards {
        *rank_counts.entry(card.rank.value()).or_insert(0) += 1;
    }

    let mut counts: Vec<_> = rank_counts.iter().collect();
    counts.sort_by(|a, b| {
        // Sort by count desc, then by rank desc
        match b.1.cmp(a.1) {
            Ordering::Equal => b.0.cmp(a.0),
            ord => ord,
        }
    });

    // Determine hand rank
    let c0 = at(&counts, 0);
    let c1 = counts.get(1);
    let c2 = counts.get(2);

    let (rank, kickers) = if is_flush && straight_high == Some(14) {
        (HandRank::RoyalFlush, vec![14, 13, 12, 11, 10])
    } else if let (true, Some(high)) = (is_flush, straight_high) {
        (HandRank::StraightFlush, vec![high])
    } else if c0.1 == &4 {
        // Four of a kind
        let quad = *c0.0;
        let kicker = c1.map_or(0, |c| *c.0);
        (HandRank::FourOfAKind, vec![quad, kicker])
    } else if c0.1 == &3 && c1.is_some_and(|c| c.1 == &2) {
        // Full house
        let trips = *c0.0;
        let pair = c1.map_or(0, |c| *c.0);
        (HandRank::FullHouse, vec![trips, pair])
    } else if is_flush {
        let kickers: Vec<u8> = sorted_cards.iter().map(|c| c.rank.value()).collect();
        (HandRank::Flush, kickers)
    } else if let Some(high) = straight_high {
        (HandRank::Straight, vec![high])
    } else if c0.1 == &3 {
        // Three of a kind
        let trips = *c0.0;
        let mut kickers = vec![trips];
        for (rank, count) in &counts {
            if **count == 1 {
                kickers.push(**rank);
            }
        }
        (HandRank::ThreeOfAKind, kickers)
    } else if c0.1 == &2 && c1.is_some_and(|c| c.1 == &2) {
        // Two pair
        let high_pair = *c0.0;
        let low_pair = c1.map_or(0, |c| *c.0);
        let kicker = c2.map_or(0, |c| *c.0);
        (HandRank::TwoPair, vec![high_pair, low_pair, kicker])
    } else if c0.1 == &2 {
        // One pair
        let pair = *c0.0;
        let mut kickers = vec![pair];
        for (rank, count) in &counts {
            if **count == 1 {
                kickers.push(**rank);
            }
        }
        (HandRank::OnePair, kickers)
    } else {
        // High card
        let kickers: Vec<u8> = sorted_cards.iter().map(|c| c.rank.value()).collect();
        (HandRank::HighCard, kickers)
    };

    EvaluatedHand {
        rank,
        cards: sorted_cards,
        kickers,
    }
}

/// Check if all cards are the same suit
fn check_flush(cards: &[Card]) -> bool {
    match cards.first() {
        Some(first) => {
            let suit = first.suit;
            cards.iter().all(|c| c.suit == suit)
        }
        None => false,
    }
}

/// Check for a straight, returns the high card value if found
fn check_straight(cards: &[Card]) -> Option<u8> {
    let mut values: Vec<u8> = cards.iter().map(|c| c.rank.value()).collect();
    values.sort_by(|a, b| b.cmp(a));
    values.dedup();

    if values.len() < 5 {
        return None;
    }

    // Check for regular straight
    for window in values.windows(5) {
        let high = *at(window, 0);
        let low = *at(window, 4);
        if high - low == 4 {
            return Some(high);
        }
    }

    // Check for wheel (A-2-3-4-5)
    if values.contains(&14)
        && values.contains(&5)
        && values.contains(&4)
        && values.contains(&3)
        && values.contains(&2)
    {
        return Some(5); // 5-high straight
    }

    None
}

/// Generate all 5-card combinations from a slice
fn combinations_5(cards: &[Card]) -> Vec<Vec<Card>> {
    let n = cards.len();
    if n < 5 {
        return vec![];
    }
    if n == 5 {
        return vec![cards.to_vec()];
    }

    let mut result = Vec::new();
    let mut indices = vec![0usize; 5];

    for i in 0..5 {
        *at_mut(&mut indices, i) = i;
    }

    loop {
        result.push(indices.iter().map(|&i| *at(cards, i)).collect());

        // Find rightmost index that can be incremented
        let mut i = 4i32;
        while i >= 0 && *at(&indices, i as usize) == n - 5 + i as usize {
            i -= 1;
        }

        if i < 0 {
            break;
        }

        *at_mut(&mut indices, i as usize) += 1;
        for j in (i + 1) as usize..5 {
            let prev = *at(&indices, j - 1);
            *at_mut(&mut indices, j) = prev + 1;
        }
    }

    result
}

/// Compare two players' hands, returns Ordering for player1 vs player2
pub fn compare_hands(hand1: &EvaluatedHand, hand2: &EvaluatedHand) -> Ordering {
    hand1.cmp(hand2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::poker::deck::{Rank, Suit};

    fn card(rank: u8, suit: Suit) -> Card {
        let rank = match rank {
            2 => Rank::Two,
            3 => Rank::Three,
            4 => Rank::Four,
            5 => Rank::Five,
            6 => Rank::Six,
            7 => Rank::Seven,
            8 => Rank::Eight,
            9 => Rank::Nine,
            10 => Rank::Ten,
            11 => Rank::Jack,
            12 => Rank::Queen,
            13 => Rank::King,
            14 => Rank::Ace,
            _ => Rank::Two,
        };
        Card::new(rank, suit)
    }

    #[test]
    fn test_royal_flush() {
        let cards = vec![
            card(14, Suit::Hearts),
            card(13, Suit::Hearts),
            card(12, Suit::Hearts),
            card(11, Suit::Hearts),
            card(10, Suit::Hearts),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::RoyalFlush);
    }

    #[test]
    fn test_straight_flush() {
        let cards = vec![
            card(9, Suit::Clubs),
            card(8, Suit::Clubs),
            card(7, Suit::Clubs),
            card(6, Suit::Clubs),
            card(5, Suit::Clubs),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::StraightFlush);
    }

    #[test]
    fn test_four_of_a_kind() {
        let cards = vec![
            card(8, Suit::Hearts),
            card(8, Suit::Diamonds),
            card(8, Suit::Clubs),
            card(8, Suit::Spades),
            card(2, Suit::Hearts),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::FourOfAKind);
    }

    #[test]
    fn test_full_house() {
        let cards = vec![
            card(10, Suit::Hearts),
            card(10, Suit::Diamonds),
            card(10, Suit::Clubs),
            card(4, Suit::Spades),
            card(4, Suit::Hearts),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::FullHouse);
    }

    #[test]
    fn test_flush() {
        let cards = vec![
            card(14, Suit::Diamonds),
            card(10, Suit::Diamonds),
            card(7, Suit::Diamonds),
            card(4, Suit::Diamonds),
            card(2, Suit::Diamonds),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::Flush);
    }

    #[test]
    fn test_straight() {
        let cards = vec![
            card(10, Suit::Hearts),
            card(9, Suit::Diamonds),
            card(8, Suit::Clubs),
            card(7, Suit::Spades),
            card(6, Suit::Hearts),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::Straight);
    }

    #[test]
    fn test_wheel_straight() {
        let cards = vec![
            card(14, Suit::Hearts),
            card(2, Suit::Diamonds),
            card(3, Suit::Clubs),
            card(4, Suit::Spades),
            card(5, Suit::Hearts),
        ];
        let hand = evaluate_hand(&cards);
        assert_eq!(hand.rank, HandRank::Straight);
        assert_eq!(hand.kickers[0], 5); // 5-high straight
    }

    #[test]
    fn test_hand_comparison() {
        let flush = evaluate_hand(&[
            card(14, Suit::Diamonds),
            card(10, Suit::Diamonds),
            card(7, Suit::Diamonds),
            card(4, Suit::Diamonds),
            card(2, Suit::Diamonds),
        ]);

        let straight = evaluate_hand(&[
            card(10, Suit::Hearts),
            card(9, Suit::Diamonds),
            card(8, Suit::Clubs),
            card(7, Suit::Spades),
            card(6, Suit::Hearts),
        ]);

        assert!(flush > straight);
    }
}
