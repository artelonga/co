//! Language type system
//!
//! Languages are subtypes of CO that provide exegetic primitives.
//! Includes i18n (internationalization) support for UI translations.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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

// ============================================================================
// Internationalization (i18n) - UI label translations
// ============================================================================

/// UI label translations for a specific language
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiLabels {
    /// Language code (e.g., "en", "pt")
    #[serde(default)]
    pub lang: String,

    /// Type labels (e.g., "definition" → "definição")
    #[serde(default)]
    pub types: HashMap<String, String>,

    /// Field labels (e.g., "status" → "estado")
    #[serde(default)]
    pub fields: HashMap<String, String>,

    /// Status labels (e.g., "active" → "ativo")
    #[serde(default)]
    pub statuses: HashMap<String, String>,

    /// Message templates (e.g., "created" → "Criado com sucesso")
    #[serde(default)]
    pub messages: HashMap<String, String>,
}

impl UiLabels {
    /// Create empty labels for a language
    pub fn new(lang: impl Into<String>) -> Self {
        Self {
            lang: lang.into(),
            ..Default::default()
        }
    }

    /// Load translations from a YAML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
    }

    /// Load translations from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("Failed to parse YAML: {}", e))
    }

    /// Get a type label with fallback to key
    pub fn type_label<'a>(&'a self, key: &'a str) -> &'a str {
        self.types.get(key).map(|s| s.as_str()).unwrap_or(key)
    }

    /// Get a field label with fallback to key
    pub fn field_label<'a>(&'a self, key: &'a str) -> &'a str {
        self.fields.get(key).map(|s| s.as_str()).unwrap_or(key)
    }

    /// Get a status label with fallback to key
    pub fn status_label<'a>(&'a self, key: &'a str) -> &'a str {
        self.statuses.get(key).map(|s| s.as_str()).unwrap_or(key)
    }

    /// Get a message with fallback to key
    pub fn message<'a>(&'a self, key: &'a str) -> &'a str {
        self.messages.get(key).map(|s| s.as_str()).unwrap_or(key)
    }
}

/// I18n manager with fallback support
#[derive(Debug, Clone)]
pub struct I18n {
    /// Current language labels
    current: UiLabels,
    /// Fallback language labels (usually English)
    fallback: UiLabels,
}

impl Default for I18n {
    fn default() -> Self {
        Self {
            current: UiLabels::new("en"),
            fallback: UiLabels::new("en"),
        }
    }
}

impl I18n {
    /// Create new I18n with current and fallback labels
    pub fn new(current: UiLabels, fallback: UiLabels) -> Self {
        Self { current, fallback }
    }

    /// Create I18n from language code, loading from i18n/{lang}.yaml
    pub fn load(lang: &str, base_path: impl AsRef<Path>) -> Result<Self, String> {
        let base = base_path.as_ref();

        // Load fallback (English)
        let fallback_path = base.join("i18n/en.yaml");
        let fallback = if fallback_path.exists() {
            UiLabels::from_file(&fallback_path)?
        } else {
            UiLabels::new("en")
        };

        // Load current language
        let current_path = base.join(format!("i18n/{}.yaml", lang));
        let current = if lang == "en" {
            fallback.clone()
        } else if current_path.exists() {
            UiLabels::from_file(&current_path)?
        } else {
            fallback.clone()
        };

        Ok(Self { current, fallback })
    }

    /// Get type label with fallback
    pub fn type_label<'a>(&'a self, key: &'a str) -> &'a str {
        let label = self.current.type_label(key);
        if label == key {
            self.fallback.type_label(key)
        } else {
            label
        }
    }

    /// Get field label with fallback
    pub fn field_label<'a>(&'a self, key: &'a str) -> &'a str {
        let label = self.current.field_label(key);
        if label == key {
            self.fallback.field_label(key)
        } else {
            label
        }
    }

    /// Get status label with fallback
    pub fn status_label<'a>(&'a self, key: &'a str) -> &'a str {
        let label = self.current.status_label(key);
        if label == key {
            self.fallback.status_label(key)
        } else {
            label
        }
    }

    /// Get message with fallback
    pub fn message<'a>(&'a self, key: &'a str) -> &'a str {
        let msg = self.current.message(key);
        if msg == key {
            self.fallback.message(key)
        } else {
            msg
        }
    }

    /// Get current language code
    pub fn lang(&self) -> &str {
        &self.current.lang
    }
}

#[cfg(test)]
mod i18n_tests {
    use super::*;

    #[test]
    fn test_i18n_loads_translations() {
        let yaml = r#"
lang: pt
types:
  definition: definição
  task: tarefa
fields:
  status: estado
messages:
  created: Criado com sucesso
"#;
        let labels = UiLabels::from_yaml(yaml).unwrap();
        assert_eq!(labels.lang, "pt");
        assert_eq!(labels.type_label("definition"), "definição");
        assert_eq!(labels.field_label("status"), "estado");
        assert_eq!(labels.message("created"), "Criado com sucesso");
    }

    #[test]
    fn test_i18n_fallback_to_key() {
        let labels = UiLabels::new("en");
        assert_eq!(labels.type_label("unknown"), "unknown");
    }

    #[test]
    fn test_i18n_fallback_to_english() {
        let en = UiLabels::from_yaml("lang: en\ntypes:\n  task: task").unwrap();
        let pt = UiLabels::from_yaml("lang: pt\ntypes:\n  definition: definição").unwrap();

        let i18n = I18n::new(pt, en);
        assert_eq!(i18n.type_label("definition"), "definição");
        assert_eq!(i18n.type_label("task"), "task");
    }
}
