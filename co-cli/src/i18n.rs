//! CLI internationalization support
//!
//! Loads UI translations based on system language configuration.
//! Falls back to embedded defaults when files are not available.
//!
//! Language configs are bundled in the binary using `include_str!()`,
//! so CO works in any external folder without requiring source files.

use co::{I18n, UiLabels};
use std::fs;
use std::path::Path;

// Embed full language configs in binary
const ENGLISH_LABELS: &str = include_str!("../../i18n/en.yaml");
const PORTUGUESE_LABELS: &str = include_str!("../../i18n/pt.yaml");

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
    i18n.message("space_initialized") != "space_initialized"
}

/// Get embedded default labels for a language
fn embedded_labels(lang: &str) -> UiLabels {
    let yaml = match lang {
        "pt" => PORTUGUESE_LABELS,
        _ => ENGLISH_LABELS,
    };

    UiLabels::from_yaml(yaml).unwrap_or_else(|_| UiLabels::new(lang))
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
    let path = format!("i18n/{}.yaml", lang);
    if Path::new(&path).exists() {
        UiLabels::from_file(&path).unwrap_or_else(|_| embedded_labels(lang))
    } else {
        embedded_labels(lang)
    }
}
