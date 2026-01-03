//! Universal type system for CO
//!
//! Type is the universal abstraction. Everything in CO inherits from Type.
//! Languages are specialized types with lexicon architecture.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The universal type abstraction.
///
/// Everything in CO is a Type. Languages, definitions, tasks, projects -
/// all inherit from the root CO type.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeKind {
    /// The root type (CO itself)
    Root,

    /// A language type with specialized metadata
    Language(LanguageSpec),

    /// A domain/project namespace
    Domain,

    /// A definition (exegetic abstraction)
    Definition,

    /// A task
    Task,

    /// A project
    Project,

    /// An agent (autonomous entity that can act on content)
    Agent,

    /// A tool (deterministic script/utility)
    Tool,

    /// Generic content
    Content,

    /// Custom user-defined type
    Custom(String),
}

/// Language-specific metadata for Language types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LanguageSpec {
    /// Type of exegesis (Natural, Formal, NonVerbal)
    pub exegesis: ExegesisType,

    /// Text direction
    pub direction: Direction,

    /// ISO 639 language code (e.g., "en", "pt")
    pub iso_code: Option<String>,

    /// Whether this is the default/template language
    pub is_default: bool,
}

/// The type of exegesis a language supports
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExegesisType {
    /// Natural human language (English, Portuguese, etc.)
    #[default]
    Natural,

    /// Formal language (Math, Logic, Programming)
    Formal,

    /// Non-verbal language (Music, Visual)
    NonVerbal,
}

impl LanguageSpec {
    /// Create a new language spec
    pub fn new(iso_code: &str, exegesis: ExegesisType, direction: Direction) -> Self {
        Self {
            exegesis,
            direction,
            iso_code: Some(iso_code.to_string()),
            is_default: false,
        }
    }

    /// Create English as the default template language
    pub fn english() -> Self {
        Self {
            exegesis: ExegesisType::Natural,
            direction: Direction::LeftToRight,
            iso_code: Some("en".to_string()),
            is_default: true,
        }
    }

    /// Create a formal language (Math, Logic, etc.)
    pub fn formal(id: &str) -> Self {
        Self {
            exegesis: ExegesisType::Formal,
            direction: Direction::None,
            iso_code: Some(id.to_string()),
            is_default: false,
        }
    }

    /// Create a non-verbal language (Music, Visual)
    pub fn non_verbal(id: &str) -> Self {
        Self {
            exegesis: ExegesisType::NonVerbal,
            direction: Direction::None,
            iso_code: Some(id.to_string()),
            is_default: false,
        }
    }
}

/// Text direction for a language
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Direction {
    /// Left to right (English, Portuguese)
    #[default]
    LeftToRight,

    /// Right to left (Arabic, Hebrew)
    RightToLeft,

    /// Top to bottom (Traditional Chinese, Japanese)
    TopToBottom,

    /// No direction (Math, Music)
    None,
}

/// A reference to a type by its identifier
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeRef(pub String);

/// Semantic version following semver.org
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SemanticVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemanticVersion {
    /// Create a new semantic version
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl std::fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A lexicon is a versioned collection of all type definitions within a scope.
///
/// If a scope (language/folder) contains 300 markdown files, the lexicon
/// will have 300 entries. All definitions are universally queryable through
/// the graph, with scopes providing namespace isolation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Lexicon {
    /// Version of this lexicon
    pub version: SemanticVersion,

    /// All entries in this lexicon, indexed by their unique ID
    /// Each entry is a reference to a definition in the graph
    pub entries: HashMap<String, LexiconEntry>,

    /// Parent lexicon this inherits from (for type extension)
    pub inherits_from: Option<TypeRef>,
}

/// An entry in a lexicon - either a definition or a reference to another term.
///
/// When querying, the content returned is always plain text from the .md file body,
/// not the YAML frontmatter or metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LexiconEntry {
    /// A definition with its plain text content (from .md body, not frontmatter)
    Definition {
        /// The plain text content of the definition
        content: String,
        /// Source file path
        file_path: String,
    },

    /// A reference to another term (e.g., for translations)
    Reference {
        /// The target term this refers to
        target: String,
        /// Optional language/scope of the target
        target_scope: Option<String>,
    },
}

impl Lexicon {
    /// Create a new empty lexicon
    pub fn new(version: SemanticVersion) -> Self {
        Self {
            version,
            entries: HashMap::new(),
            inherits_from: None,
        }
    }

    /// Define a term with its plain text content
    pub fn define(&mut self, name: &str, content: &str, file_path: &str) {
        self.entries.insert(
            name.to_string(),
            LexiconEntry::Definition {
                content: content.to_string(),
                file_path: file_path.to_string(),
            },
        );
    }

    /// Add a reference (translation) to another term
    pub fn reference(&mut self, name: &str, target: &str, target_scope: Option<&str>) {
        self.entries.insert(
            name.to_string(),
            LexiconEntry::Reference {
                target: target.to_string(),
                target_scope: target_scope.map(String::from),
            },
        );
    }

    /// Get the number of entries in this lexicon
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if lexicon is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get an entry by name
    pub fn get(&self, name: &str) -> Option<&LexiconEntry> {
        self.entries.get(name)
    }

    /// Get the plain text content for a term (resolving references if needed)
    pub fn content(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            LexiconEntry::Definition { content, .. } => Some(content),
            LexiconEntry::Reference { .. } => None, // Caller must resolve references
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexicon_definitions_return_plain_text() {
        // Lexicon entries store plain text content from .md body (not frontmatter)
        let mut lexicon = Lexicon::new(SemanticVersion::new(1, 0, 0));

        // Define terms with their plain text content
        lexicon.define(
            "hello",
            "A common greeting used when meeting someone.",
            "en/definitions/hello.md",
        );

        lexicon.define(
            "world",
            "The earth and all its inhabitants.",
            "en/definitions/world.md",
        );

        assert_eq!(lexicon.len(), 2);

        // Query returns plain text content, not metadata
        assert_eq!(
            lexicon.content("hello"),
            Some("A common greeting used when meeting someone.")
        );
    }

    #[test]
    fn test_lexicon_references_for_translations() {
        // References point to terms in other lexicons (translations)
        let mut pt_lexicon = Lexicon::new(SemanticVersion::new(1, 0, 0));

        // Portuguese "olá" references English "hello"
        pt_lexicon.reference("olá", "hello", Some("en"));

        // Can also have native definitions
        pt_lexicon.define(
            "saudade",
            "A deep emotional state of longing for something absent.",
            "pt/definitions/saudade.md",
        );

        assert_eq!(pt_lexicon.len(), 2);

        // References don't return content directly (must be resolved)
        assert!(pt_lexicon.content("olá").is_none());
        assert!(pt_lexicon.content("saudade").is_some());
    }

    #[test]
    fn test_lexicon_private_individual_scope() {
        // EXAMPLE: "private" scope for individual/personal definitions
        let mut private_lexicon = Lexicon::new(SemanticVersion::new(1, 0, 0));

        private_lexicon.define(
            "daily-standup",
            "Quick morning sync to share progress and blockers.",
            "private/definitions/daily-standup.md",
        );

        private_lexicon.define(
            "personal-okrs",
            "Quarterly objectives and key results for personal growth.",
            "private/definitions/personal-okrs.md",
        );

        assert_eq!(private_lexicon.len(), 2);
        assert!(private_lexicon.content("daily-standup").is_some());
    }

    #[test]
    fn test_lexicon_org_collective_scope() {
        // EXAMPLE: "org" scope for organizational/collective definitions
        let mut org_lexicon = Lexicon::new(SemanticVersion::new(1, 0, 0));

        org_lexicon.define(
            "sprint",
            "A two-week iteration for delivering incremental value.",
            "org/definitions/sprint.md",
        );

        org_lexicon.define(
            "retro",
            "Team reflection meeting at the end of each sprint.",
            "org/definitions/retro.md",
        );

        assert_eq!(org_lexicon.len(), 2);

        // Scopes are isolated - org terms not in private scope
        let private_lexicon = Lexicon::new(SemanticVersion::new(1, 0, 0));
        assert!(private_lexicon.get("sprint").is_none());
    }

    #[test]
    fn test_semantic_version() {
        let version = SemanticVersion::new(2, 1, 3);
        assert_eq!(version.to_string(), "2.1.3");
    }

    #[test]
    fn test_language_spec_english_default() {
        // English should be the default template language
        let english = LanguageSpec::english();

        assert!(english.is_default);
        assert_eq!(english.iso_code, Some("en".to_string()));
        assert_eq!(english.exegesis, ExegesisType::Natural);
        assert_eq!(english.direction, Direction::LeftToRight);
    }

    #[test]
    fn test_language_spec_portuguese() {
        let portuguese = LanguageSpec::new("pt", ExegesisType::Natural, Direction::LeftToRight);

        assert!(!portuguese.is_default);
        assert_eq!(portuguese.iso_code, Some("pt".to_string()));
    }

    #[test]
    fn test_language_spec_formal_math() {
        let math = LanguageSpec::formal("math");

        assert_eq!(math.exegesis, ExegesisType::Formal);
        assert_eq!(math.direction, Direction::None);
    }

    #[test]
    fn test_typekind_variants() {
        // Test that all TypeKind variants can be constructed
        let root = TypeKind::Root;
        let lang = TypeKind::Language(LanguageSpec::default());
        let domain = TypeKind::Domain;
        let def = TypeKind::Definition;
        let task = TypeKind::Task;
        let project = TypeKind::Project;
        let agent = TypeKind::Agent;
        let tool = TypeKind::Tool;
        let content = TypeKind::Content;
        let custom = TypeKind::Custom("my-type".to_string());

        // Verify enum variants exist and pattern matching works
        assert!(matches!(root, TypeKind::Root));
        assert!(matches!(lang, TypeKind::Language(_)));
        assert!(matches!(domain, TypeKind::Domain));
        assert!(matches!(def, TypeKind::Definition));
        assert!(matches!(task, TypeKind::Task));
        assert!(matches!(project, TypeKind::Project));
        assert!(matches!(agent, TypeKind::Agent));
        assert!(matches!(tool, TypeKind::Tool));
        assert!(matches!(content, TypeKind::Content));
        assert!(matches!(custom, TypeKind::Custom(_)));
    }

    #[test]
    fn test_typekind_clone_and_debug() {
        let kind = TypeKind::Task;
        let cloned = kind.clone();
        assert_eq!(format!("{:?}", kind), format!("{:?}", cloned));
    }

    #[test]
    fn test_typekind_serialization() {
        let kind = TypeKind::Definition;
        let json = serde_json::to_string(&kind).unwrap();
        let deserialized: TypeKind = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, TypeKind::Definition));
    }
}
