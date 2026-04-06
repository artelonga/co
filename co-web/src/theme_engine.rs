//! CO-30 — Dynamic CSS engine: token generation from universe config at runtime.
//!
//! This module defines [`ThemePreset`] (a named set of CSS design tokens) and
//! [`generate_css`] (renders a complete `:root { … }` stylesheet from a preset
//! with optional per-universe custom token overrides merged on top).

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A named theme with a full set of CSS design tokens and optional font metadata.
///
/// Tokens are plain CSS custom-property names (e.g. `"--bg"`) mapped to their
/// values (e.g. `"#FFF9ED"`).  Font fields are the Google Fonts family names
/// used by the headline, body, and label roles.
#[derive(Debug, Clone)]
pub struct ThemePreset {
    pub name: String,
    pub tokens: HashMap<String, String>,
    pub font_headline: Option<String>,
    pub font_body: Option<String>,
    pub font_label: Option<String>,
}

impl ThemePreset {
    fn new(name: &str) -> Self {
        ThemePreset {
            name: name.to_string(),
            tokens: HashMap::new(),
            font_headline: None,
            font_body: None,
            font_label: None,
        }
    }

    /// Returns all built-in presets.
    pub fn all_presets() -> Vec<ThemePreset> {
        vec![
            scholarly_preset(),
            scholarly_dark_preset(),
            relic_preset(),
            relic_light_preset(),
            modern_preset(),
        ]
    }

    /// Look up a preset by name (also handles the `"scholarly-light"` alias and
    /// the empty-string default for the `modern` preset).
    pub fn by_name(name: &str) -> Option<ThemePreset> {
        match name {
            "scholarly" | "scholarly-light" => Some(scholarly_preset()),
            "scholarly-dark" => Some(scholarly_dark_preset()),
            "relic" => Some(relic_preset()),
            "relic-light" => Some(relic_light_preset()),
            "modern" | "" => Some(modern_preset()),
            _ => None,
        }
    }

    /// Returns the companion preset name for dark ↔ light toggling.
    ///
    /// Pairs: `scholarly` ↔ `scholarly-dark`, `relic-light` ↔ `relic`,
    /// `modern` stays `modern` (single-variant).
    pub fn companion_name(name: &str) -> &'static str {
        match name {
            "scholarly" | "scholarly-light" => "scholarly-dark",
            "scholarly-dark" => "scholarly",
            "relic" => "relic-light",
            "relic-light" => "relic",
            _ => "scholarly",
        }
    }

    /// Returns `true` when the preset is a dark-background variant.
    pub fn is_dark(name: &str) -> bool {
        matches!(name, "scholarly-dark" | "relic")
    }
}

// ---------------------------------------------------------------------------
// CSS generation
// ---------------------------------------------------------------------------

/// Render a complete `:root { … }` CSS block from *preset* with *overrides*
/// merged on top.
///
/// Tokens are sorted alphabetically so the output is deterministic and safe to
/// hash for ETags.  Override keys must be valid CSS custom-property names
/// (starting with `--`); non-string JSON values are silently ignored.
pub fn generate_css(preset: &ThemePreset, overrides: Option<&serde_json::Value>) -> String {
    let mut tokens = preset.tokens.clone();

    if let Some(obj) = overrides.and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                tokens.insert(k.clone(), s.to_string());
            }
        }
    }

    // Alphabetic sort → deterministic output (ETag / cache stability).
    let mut sorted: Vec<(String, String)> = tokens.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let declarations = sorted
        .iter()
        .map(|(k, v)| format!("  {k}: {v};"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(":root {{\n{declarations}\n}}\n")
}

// ---------------------------------------------------------------------------
// Built-in preset definitions
// ---------------------------------------------------------------------------

fn scholarly_preset() -> ThemePreset {
    let mut p = ThemePreset::new("scholarly");
    p.font_headline = Some("Newsreader".into());
    p.font_body = Some("Work Sans".into());
    p.font_label = Some("Work Sans".into());
    let t = &mut p.tokens;
    t.insert("--bg".into(), "#FFF9ED".into());
    t.insert("--bg-hover".into(), "#F3EDE1".into());
    t.insert("--sidebar-bg".into(), "#F2E8D5".into());
    t.insert("--sidebar-hover".into(), "#EAD9BF".into());
    t.insert("--sidebar-active".into(), "#CD7F32".into());
    t.insert("--card-bg".into(), "#FFFFFF".into());
    t.insert("--border".into(), "rgba(134,116,102,0.15)".into());
    t.insert("--text-primary".into(), "#1d1c15".into());
    t.insert("--text-secondary".into(), "#534438".into());
    t.insert("--text-muted".into(), "#867466".into());
    t.insert("--accent".into(), "#8E4E00".into());
    t.insert("--accent-hover".into(), "#6c3a00".into());
    t.insert("--accent-light".into(), "#ffdcc1".into());
    t.insert("--danger".into(), "#ba1a1a".into());
    t.insert("--danger-hover".into(), "#93000a".into());
    t.insert("--font".into(), "'Newsreader', Georgia, serif".into());
    t.insert(
        "--font-label".into(),
        "'Work Sans', system-ui, sans-serif".into(),
    );
    t.insert(
        "--font-mono".into(),
        "'SF Mono', 'Cascadia Mono', Consolas, monospace".into(),
    );
    t.insert("--radius-sm".into(), "2px".into());
    t.insert("--radius-md".into(), "4px".into());
    t.insert("--radius-lg".into(), "8px".into());
    t.insert("--shadow-sm".into(), "0 1px 3px rgba(142,78,0,0.06)".into());
    t.insert(
        "--shadow-md".into(),
        "0 4px 12px rgba(142,78,0,0.08), 0 2px 4px rgba(142,78,0,0.05)".into(),
    );
    t.insert(
        "--shadow-lg".into(),
        "0 12px 40px rgba(142,78,0,0.12), 0 4px 8px rgba(142,78,0,0.06)".into(),
    );
    t.insert("--modal-overlay".into(), "rgba(29,28,21,0.45)".into());
    t.insert("--modal-surface".into(), "#FFFEF9".into());
    t.insert("--form-input-bg".into(), "transparent".into());
    t.insert("--form-input-border".into(), "none".into());
    t.insert("--form-input-border-focus".into(), "none".into());
    t.insert("--form-input-radius".into(), "0".into());
    t.insert("--form-input-padding".into(), "8px 0".into());
    insert_default_status_tokens(t);
    insert_default_priority_tokens(t);
    p
}

fn scholarly_dark_preset() -> ThemePreset {
    let mut p = ThemePreset::new("scholarly-dark");
    p.font_headline = Some("Newsreader".into());
    p.font_body = Some("Work Sans".into());
    p.font_label = Some("Work Sans".into());
    let t = &mut p.tokens;
    t.insert("--bg".into(), "#1c1610".into());
    t.insert("--bg-hover".into(), "#221810".into());
    t.insert("--sidebar-bg".into(), "#0f0b07".into());
    t.insert("--sidebar-hover".into(), "#1c1408".into());
    t.insert("--sidebar-active".into(), "#CD7F32".into());
    t.insert("--card-bg".into(), "#271c12".into());
    t.insert("--border".into(), "rgba(205,127,50,0.2)".into());
    t.insert("--text-primary".into(), "#f0e0c8".into());
    t.insert("--text-secondary".into(), "#c4986a".into());
    t.insert("--text-muted".into(), "#9a7a54".into());
    t.insert("--accent".into(), "#CD7F32".into());
    t.insert("--accent-hover".into(), "#ffdcc1".into());
    t.insert("--accent-light".into(), "#4a2a00".into());
    t.insert("--danger".into(), "#ff6b6b".into());
    t.insert("--danger-hover".into(), "#ff4040".into());
    t.insert("--font".into(), "'Newsreader', Georgia, serif".into());
    t.insert(
        "--font-label".into(),
        "'Work Sans', system-ui, sans-serif".into(),
    );
    t.insert(
        "--font-mono".into(),
        "'SF Mono', 'Cascadia Mono', Consolas, monospace".into(),
    );
    t.insert("--radius-sm".into(), "2px".into());
    t.insert("--radius-md".into(), "4px".into());
    t.insert("--radius-lg".into(), "8px".into());
    t.insert("--shadow-sm".into(), "0 1px 3px rgba(0,0,0,0.3)".into());
    t.insert("--shadow-md".into(), "0 4px 12px rgba(0,0,0,0.4)".into());
    t.insert("--shadow-lg".into(), "0 12px 40px rgba(0,0,0,0.5)".into());
    t.insert("--modal-overlay".into(), "rgba(0,0,0,0.65)".into());
    t.insert("--modal-surface".into(), "#2e2116".into());
    t.insert("--form-input-bg".into(), "transparent".into());
    t.insert("--form-input-border".into(), "none".into());
    t.insert("--form-input-border-focus".into(), "none".into());
    t.insert("--form-input-radius".into(), "0".into());
    t.insert("--form-input-padding".into(), "8px 0".into());
    t.insert("--status-todo".into(), "#7d6040".into());
    t.insert("--status-in_progress".into(), "#CD7F32".into());
    t.insert("--status-in_review".into(), "#e09020".into());
    t.insert("--status-done".into(), "#6a9a50".into());
    t.insert("--status-todo-bg".into(), "#261d14".into());
    t.insert("--status-in_progress-bg".into(), "#3d2000".into());
    t.insert("--status-in_review-bg".into(), "#3d2800".into());
    t.insert("--status-done-bg".into(), "#1a2e14".into());
    t.insert("--status-todo-text".into(), "#b89070".into());
    t.insert("--status-in_progress-text".into(), "#CD7F32".into());
    t.insert("--status-in_review-text".into(), "#ffd090".into());
    t.insert("--status-done-text".into(), "#90c070".into());
    insert_default_priority_tokens(t);
    p
}

fn relic_preset() -> ThemePreset {
    let mut p = ThemePreset::new("relic");
    p.font_headline = Some("Newsreader".into());
    p.font_body = Some("Manrope".into());
    p.font_label = Some("Manrope".into());
    let t = &mut p.tokens;
    t.insert("--bg".into(), "#131313".into());
    t.insert("--bg-hover".into(), "#222222".into());
    t.insert("--sidebar-bg".into(), "#0d0d0d".into());
    t.insert("--sidebar-hover".into(), "#1a1919".into());
    t.insert("--sidebar-active".into(), "#e0505f".into());
    t.insert("--card-bg".into(), "#262424".into());
    t.insert("--border".into(), "rgba(224,80,95,0.18)".into());
    t.insert("--text-primary".into(), "#e5e2e1".into());
    t.insert("--text-secondary".into(), "#c5c6cc".into());
    t.insert("--text-muted".into(), "#8f9096".into());
    t.insert("--accent".into(), "#e0505f".into());
    t.insert("--accent-hover".into(), "#ffb3b5".into());
    t.insert("--accent-light".into(), "#3d000a".into());
    t.insert("--danger".into(), "#ff6b6b".into());
    t.insert("--danger-hover".into(), "#ff4040".into());
    t.insert("--font".into(), "'Newsreader', Georgia, serif".into());
    t.insert(
        "--font-label".into(),
        "'Manrope', system-ui, sans-serif".into(),
    );
    t.insert(
        "--font-mono".into(),
        "'SF Mono', 'Cascadia Mono', Consolas, monospace".into(),
    );
    t.insert("--radius-sm".into(), "2px".into());
    t.insert("--radius-md".into(), "4px".into());
    t.insert("--radius-lg".into(), "8px".into());
    t.insert("--shadow-sm".into(), "0 1px 3px rgba(0,0,0,0.4)".into());
    t.insert("--shadow-md".into(), "0 4px 16px rgba(0,0,0,0.5)".into());
    t.insert("--shadow-lg".into(), "0 12px 50px rgba(0,0,0,0.7)".into());
    t.insert("--modal-overlay".into(), "rgba(0,0,0,0.75)".into());
    t.insert("--modal-surface".into(), "#2e2c2c".into());
    t.insert("--form-input-bg".into(), "rgba(38,36,36,0.6)".into());
    t.insert(
        "--form-input-border".into(),
        "1px solid rgba(224,80,95,0.2)".into(),
    );
    t.insert(
        "--form-input-border-focus".into(),
        "1px solid #e9c349".into(),
    );
    t.insert("--form-input-radius".into(), "4px".into());
    t.insert("--form-input-padding".into(), "8px 10px".into());
    t.insert("--status-todo".into(), "#8f9096".into());
    t.insert("--status-in_progress".into(), "#e9c349".into());
    t.insert("--status-in_review".into(), "#e0505f".into());
    t.insert("--status-done".into(), "#6a9a50".into());
    t.insert("--status-todo-bg".into(), "#1c1b1b".into());
    t.insert("--status-in_progress-bg".into(), "#2a2500".into());
    t.insert("--status-in_review-bg".into(), "#2a0a0e".into());
    t.insert("--status-done-bg".into(), "#0e2010".into());
    t.insert("--status-todo-text".into(), "#8f9096".into());
    t.insert("--status-in_progress-text".into(), "#e9c349".into());
    t.insert("--status-in_review-text".into(), "#ffb3b5".into());
    t.insert("--status-done-text".into(), "#90c070".into());
    insert_default_priority_tokens(t);
    p
}

fn relic_light_preset() -> ThemePreset {
    let mut p = ThemePreset::new("relic-light");
    p.font_headline = Some("Newsreader".into());
    p.font_body = Some("Manrope".into());
    p.font_label = Some("Manrope".into());
    let t = &mut p.tokens;
    t.insert("--bg".into(), "#F5F0F0".into());
    t.insert("--bg-hover".into(), "#EDE5E5".into());
    t.insert("--sidebar-bg".into(), "#E8DEDE".into());
    t.insert("--sidebar-hover".into(), "#DDD0D0".into());
    t.insert("--sidebar-active".into(), "#af2b3e".into());
    t.insert("--card-bg".into(), "#FFFFFF".into());
    t.insert("--border".into(), "rgba(175,43,62,0.1)".into());
    t.insert("--text-primary".into(), "#1a1010".into());
    t.insert("--text-secondary".into(), "#5a3a3a".into());
    t.insert("--text-muted".into(), "#8a6060".into());
    t.insert("--accent".into(), "#af2b3e".into());
    t.insert("--accent-hover".into(), "#8e1020".into());
    t.insert("--accent-light".into(), "#ffdada".into());
    t.insert("--danger".into(), "#ba1a1a".into());
    t.insert("--danger-hover".into(), "#93000a".into());
    t.insert("--font".into(), "'Newsreader', Georgia, serif".into());
    t.insert(
        "--font-label".into(),
        "'Manrope', system-ui, sans-serif".into(),
    );
    t.insert(
        "--font-mono".into(),
        "'SF Mono', 'Cascadia Mono', Consolas, monospace".into(),
    );
    t.insert("--radius-sm".into(), "2px".into());
    t.insert("--radius-md".into(), "4px".into());
    t.insert("--radius-lg".into(), "8px".into());
    t.insert(
        "--shadow-sm".into(),
        "0 1px 3px rgba(175,43,62,0.06)".into(),
    );
    t.insert(
        "--shadow-md".into(),
        "0 4px 12px rgba(175,43,62,0.08)".into(),
    );
    t.insert(
        "--shadow-lg".into(),
        "0 12px 40px rgba(175,43,62,0.12)".into(),
    );
    t.insert("--modal-overlay".into(), "rgba(26,16,16,0.45)".into());
    t.insert("--modal-surface".into(), "#FFFFFF".into());
    t.insert("--form-input-bg".into(), "transparent".into());
    t.insert(
        "--form-input-border".into(),
        "1px solid rgba(175,43,62,0.2)".into(),
    );
    t.insert(
        "--form-input-border-focus".into(),
        "1px solid #af2b3e".into(),
    );
    t.insert("--form-input-radius".into(), "4px".into());
    t.insert("--form-input-padding".into(), "8px 10px".into());
    insert_default_status_tokens(t);
    insert_default_priority_tokens(t);
    p
}

fn modern_preset() -> ThemePreset {
    let mut p = ThemePreset::new("modern");
    let t = &mut p.tokens;
    t.insert("--bg".into(), "#f0f2f5".into());
    t.insert("--bg-hover".into(), "#e8eaed".into());
    t.insert("--sidebar-bg".into(), "#1a1d23".into());
    t.insert("--sidebar-hover".into(), "#252830".into());
    t.insert("--sidebar-active".into(), "#6366f1".into());
    t.insert("--card-bg".into(), "#ffffff".into());
    t.insert("--border".into(), "#e5e7eb".into());
    t.insert("--text-primary".into(), "#111827".into());
    t.insert("--text-secondary".into(), "#6b7280".into());
    t.insert("--text-muted".into(), "#9ca3af".into());
    t.insert("--accent".into(), "#6366f1".into());
    t.insert("--accent-hover".into(), "#4f46e5".into());
    t.insert("--accent-light".into(), "#e0e7ff".into());
    t.insert("--danger".into(), "#ef4444".into());
    t.insert("--danger-hover".into(), "#dc2626".into());
    t.insert(
        "--font".into(),
        "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
            .into(),
    );
    t.insert("--font-label".into(), "var(--font)".into());
    t.insert(
        "--font-mono".into(),
        "'SF Mono', 'Cascadia Mono', Consolas, monospace".into(),
    );
    t.insert("--radius-sm".into(), "4px".into());
    t.insert("--radius-md".into(), "8px".into());
    t.insert("--radius-lg".into(), "12px".into());
    t.insert("--shadow-sm".into(), "0 1px 2px rgba(0,0,0,0.05)".into());
    t.insert(
        "--shadow-md".into(),
        "0 4px 6px -1px rgba(0,0,0,0.07), 0 2px 4px -2px rgba(0,0,0,0.05)".into(),
    );
    t.insert(
        "--shadow-lg".into(),
        "0 10px 15px -3px rgba(0,0,0,0.08), 0 4px 6px -4px rgba(0,0,0,0.05)".into(),
    );
    t.insert("--modal-overlay".into(), "rgba(0,0,0,0.4)".into());
    t.insert("--modal-surface".into(), "var(--card-bg)".into());
    t.insert("--form-input-bg".into(), "white".into());
    t.insert(
        "--form-input-border".into(),
        "1px solid var(--border)".into(),
    );
    t.insert(
        "--form-input-border-focus".into(),
        "1px solid var(--accent)".into(),
    );
    t.insert("--form-input-radius".into(), "var(--radius-sm)".into());
    t.insert("--form-input-padding".into(), "8px 10px".into());
    insert_default_status_tokens(t);
    insert_default_priority_tokens(t);
    p
}

// ---------------------------------------------------------------------------
// Shared token helpers
// ---------------------------------------------------------------------------

fn insert_default_status_tokens(t: &mut HashMap<String, String>) {
    t.insert("--status-todo".into(), "#94a3b8".into());
    t.insert("--status-in_progress".into(), "#3b82f6".into());
    t.insert("--status-in_review".into(), "#f59e0b".into());
    t.insert("--status-done".into(), "#22c55e".into());
    t.insert("--status-todo-bg".into(), "#f1f5f9".into());
    t.insert("--status-in_progress-bg".into(), "#dbeafe".into());
    t.insert("--status-in_review-bg".into(), "#fef3c7".into());
    t.insert("--status-done-bg".into(), "#dcfce7".into());
    t.insert("--status-todo-text".into(), "#475569".into());
    t.insert("--status-in_progress-text".into(), "#1d4ed8".into());
    t.insert("--status-in_review-text".into(), "#92400e".into());
    t.insert("--status-done-text".into(), "#166534".into());
}

fn insert_default_priority_tokens(t: &mut HashMap<String, String>) {
    t.insert("--priority-low".into(), "#94a3b8".into());
    t.insert("--priority-medium".into(), "#3b82f6".into());
    t.insert("--priority-high".into(), "#f59e0b".into());
    t.insert("--priority-critical".into(), "#ef4444".into());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Required tokens that every preset must define (pub for cross-module test use).
    pub const REQUIRED_TOKENS: &[&str] = &[
        "--bg",
        "--sidebar-bg",
        "--card-bg",
        "--text-primary",
        "--text-secondary",
        "--accent",
        "--border",
        "--status-todo",
        "--status-in_progress",
        "--status-in_review",
        "--status-done",
        "--status-todo-bg",
        "--status-in_progress-bg",
        "--status-in_review-bg",
        "--status-done-bg",
        "--status-todo-text",
        "--status-in_progress-text",
        "--status-in_review-text",
        "--status-done-text",
        "--priority-low",
        "--priority-medium",
        "--priority-high",
        "--priority-critical",
        "--font",
        "--font-mono",
        "--radius-sm",
        "--radius-md",
        "--radius-lg",
        "--shadow-sm",
        "--shadow-md",
        "--shadow-lg",
    ];

    #[test]
    fn all_presets_define_required_tokens() {
        for preset in ThemePreset::all_presets() {
            for token in REQUIRED_TOKENS {
                assert!(
                    preset.tokens.contains_key(*token),
                    "Preset '{}' is missing required token '{}'",
                    preset.name,
                    token
                );
            }
        }
    }

    #[test]
    fn generate_css_contains_root_block() {
        let preset = ThemePreset::by_name("scholarly").unwrap();
        let css = generate_css(&preset, None);
        assert!(css.starts_with(":root {"), "CSS must start with ':root {{'");
        assert!(css.contains("--bg:"), "CSS must contain --bg");
        assert!(css.contains("--accent:"), "CSS must contain --accent");
    }

    #[test]
    fn generate_css_scholarly_bg_correct() {
        let preset = ThemePreset::by_name("scholarly").unwrap();
        let css = generate_css(&preset, None);
        assert!(
            css.contains("--bg: #FFF9ED;"),
            "scholarly --bg must be #FFF9ED"
        );
    }

    #[test]
    fn generate_css_scholarly_dark_bg_correct() {
        let preset = ThemePreset::by_name("scholarly-dark").unwrap();
        let css = generate_css(&preset, None);
        assert!(
            css.contains("--bg: #1c1610;"),
            "scholarly-dark --bg must be #1c1610"
        );
    }

    #[test]
    fn generate_css_relic_accent_correct() {
        let preset = ThemePreset::by_name("relic").unwrap();
        let css = generate_css(&preset, None);
        assert!(
            css.contains("--accent: #e0505f;"),
            "relic --accent must be #e0505f"
        );
    }

    #[test]
    fn generate_css_custom_overrides_merge() {
        let preset = ThemePreset::by_name("modern").unwrap();
        let overrides = serde_json::json!({ "--accent": "#ff0000", "--bg": "#000000" });
        let css = generate_css(&preset, Some(&overrides));
        assert!(
            css.contains("--accent: #ff0000;"),
            "override --accent must win"
        );
        assert!(css.contains("--bg: #000000;"), "override --bg must win");
        // Other tokens remain from preset.
        assert!(css.contains("--radius-sm:"), "preset tokens still present");
    }

    #[test]
    fn generate_css_is_deterministic() {
        let preset = ThemePreset::by_name("relic-light").unwrap();
        let css1 = generate_css(&preset, None);
        let css2 = generate_css(&preset, None);
        assert_eq!(css1, css2, "generate_css must be deterministic");
    }

    #[test]
    fn by_name_handles_alias_scholarly_light() {
        let a = ThemePreset::by_name("scholarly").unwrap();
        let b = ThemePreset::by_name("scholarly-light").unwrap();
        assert_eq!(a.name, b.name);
    }

    #[test]
    fn by_name_handles_empty_string_as_modern() {
        let preset = ThemePreset::by_name("").unwrap();
        assert_eq!(preset.name, "modern");
    }

    #[test]
    fn by_name_returns_none_for_unknown() {
        assert!(ThemePreset::by_name("nonexistent").is_none());
    }

    #[test]
    fn companion_name_pairs_are_symmetric() {
        let pairs = [
            ("scholarly", "scholarly-dark"),
            ("scholarly-dark", "scholarly"),
            ("relic", "relic-light"),
            ("relic-light", "relic"),
        ];
        for (a, b) in pairs {
            assert_eq!(ThemePreset::companion_name(a), b);
            assert_eq!(ThemePreset::companion_name(b), a);
        }
    }

    #[test]
    fn is_dark_correct() {
        assert!(ThemePreset::is_dark("scholarly-dark"));
        assert!(ThemePreset::is_dark("relic"));
        assert!(!ThemePreset::is_dark("scholarly"));
        assert!(!ThemePreset::is_dark("relic-light"));
        assert!(!ThemePreset::is_dark("modern"));
    }

    /// Changing the theme produces different CSS output.
    #[test]
    fn different_themes_produce_different_css() {
        let scholarly = ThemePreset::by_name("scholarly").unwrap();
        let relic = ThemePreset::by_name("relic").unwrap();
        let css_scholarly = generate_css(&scholarly, None);
        let css_relic = generate_css(&relic, None);
        assert_ne!(css_scholarly, css_relic, "different themes must differ");
        // Key discriminating token
        assert!(css_scholarly.contains("#FFF9ED"), "scholarly --bg");
        assert!(css_relic.contains("#131313"), "relic --bg");
    }
}
