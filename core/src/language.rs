//! Language type system
//!
//! Languages are subtypes of CO that provide exegetic primitives.

use serde::{Deserialize, Serialize};

/// A language in the CO type system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    /// Unique identifier (e.g., "english", "portuguese")
    pub id: String,

    /// Display name
    pub name: String,

    /// ISO 639-1 code if applicable
    pub iso_code: Option<String>,

    /// Type of exegesis this language provides
    pub exegesis_type: ExegesisType,

    /// Writing direction
    pub direction: Direction,
}

/// The type of exegesis a language provides
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExegesisType {
    /// Natural language (cultural exegesis)
    Natural,

    /// Formal/symbolic language (formal exegesis)
    Formal,

    /// Non-verbal language (non-verbal exegesis)
    NonVerbal,
}

/// Writing direction
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    None, // For non-textual languages like music
}

impl Language {
    /// Create a new natural language
    pub fn natural(id: &str, name: &str, iso_code: Option<&str>) -> Self {
        Language {
            id: id.to_string(),
            name: name.to_string(),
            iso_code: iso_code.map(String::from),
            exegesis_type: ExegesisType::Natural,
            direction: Direction::LeftToRight,
        }
    }

    /// Create a formal language (like Math)
    pub fn formal(id: &str, name: &str) -> Self {
        Language {
            id: id.to_string(),
            name: name.to_string(),
            iso_code: None,
            exegesis_type: ExegesisType::Formal,
            direction: Direction::LeftToRight,
        }
    }

    /// Create a non-verbal language (like Music)
    pub fn non_verbal(id: &str, name: &str) -> Self {
        Language {
            id: id.to_string(),
            name: name.to_string(),
            iso_code: None,
            exegesis_type: ExegesisType::NonVerbal,
            direction: Direction::None,
        }
    }

    /// The five initial CO languages
    pub fn initial_languages() -> Vec<Language> {
        vec![
            Language::natural("english", "English", Some("en")),
            Language::natural("portuguese", "Portuguese", Some("pt")),
            Language::natural("guarani-mbya", "Guarani Mbya", Some("gun")),
            Language::non_verbal("music", "Music"),
            Language::formal("math", "Mathematics"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_natural_language() {
        let lang = Language::natural("english", "English", Some("en"));
        assert_eq!(lang.id, "english");
        assert_eq!(lang.name, "English");
        assert_eq!(lang.iso_code, Some("en".to_string()));
        assert_eq!(lang.exegesis_type, ExegesisType::Natural);
        assert_eq!(lang.direction, Direction::LeftToRight);
    }

    #[test]
    fn test_formal_language() {
        let lang = Language::formal("math", "Mathematics");
        assert_eq!(lang.id, "math");
        assert!(lang.iso_code.is_none());
        assert_eq!(lang.exegesis_type, ExegesisType::Formal);
    }

    #[test]
    fn test_non_verbal_language() {
        let lang = Language::non_verbal("music", "Music");
        assert_eq!(lang.exegesis_type, ExegesisType::NonVerbal);
        assert_eq!(lang.direction, Direction::None);
    }

    #[test]
    fn test_initial_languages_count() {
        let langs = Language::initial_languages();
        assert_eq!(langs.len(), 5);
    }

    #[test]
    fn test_initial_languages_types() {
        let langs = Language::initial_languages();
        let natural_count = langs
            .iter()
            .filter(|l| l.exegesis_type == ExegesisType::Natural)
            .count();
        let formal_count = langs
            .iter()
            .filter(|l| l.exegesis_type == ExegesisType::Formal)
            .count();
        let non_verbal_count = langs
            .iter()
            .filter(|l| l.exegesis_type == ExegesisType::NonVerbal)
            .count();

        assert_eq!(natural_count, 3);
        assert_eq!(formal_count, 1);
        assert_eq!(non_verbal_count, 1);
    }
}
