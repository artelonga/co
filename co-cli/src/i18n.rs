//! CLI internationalization support
//!
//! Loads UI translations based on system language configuration.
//! Falls back to embedded defaults when files are not available.

use co::{I18n, UiLabels};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Load I18n based on system configuration
pub fn load_i18n() -> I18n {
    let lang = get_system_language();

    // Try loading from files first, fall back to embedded defaults
    match I18n::load(&lang, ".") {
        Ok(i18n) if has_translations(&i18n) => i18n,
        _ => I18n::new(embedded_labels(&lang), embedded_labels("en")),
    }
}

/// Check if loaded I18n has actual translations
fn has_translations(i18n: &I18n) -> bool {
    // If we can get a translated message, we have translations
    i18n.message("context_initialized") != "context_initialized"
}

/// Get embedded default labels for a language
fn embedded_labels(lang: &str) -> UiLabels {
    let mut labels = UiLabels::new(lang);

    // Add default English labels
    labels.types = HashMap::from([
        (
            "context".to_string(),
            if lang == "pt" {
                "contexto".to_string()
            } else {
                "context".to_string()
            },
        ),
        (
            "definition".to_string(),
            if lang == "pt" {
                "definição".to_string()
            } else {
                "definition".to_string()
            },
        ),
    ]);

    labels.messages = HashMap::from([
        (
            "context_initialized".to_string(),
            if lang == "pt" {
                "Contexto Inicializado".to_string()
            } else {
                "Context Initialized".to_string()
            },
        ),
        (
            "already_exists".to_string(),
            if lang == "pt" {
                "já existe".to_string()
            } else {
                "already exists".to_string()
            },
        ),
    ]);

    labels
}

/// Get the configured system language, defaulting to "en"
fn get_system_language() -> String {
    let config_path = Path::new(".co/config.yaml");

    if config_path.exists()
        && let Ok(content) = fs::read_to_string(config_path)
    {
        for line in content.lines() {
            if line.starts_with("system_language:") {
                return line
                    .strip_prefix("system_language:")
                    .unwrap()
                    .trim()
                    .to_string();
            }
        }
    }

    "en".to_string()
}

/// Load UI labels for a specific language
#[allow(dead_code)]
pub fn load_labels(lang: &str) -> UiLabels {
    let path = format!("ui/{}.yaml", lang);
    if Path::new(&path).exists() {
        UiLabels::from_file(&path).unwrap_or_else(|_| embedded_labels(lang))
    } else {
        embedded_labels(lang)
    }
}
