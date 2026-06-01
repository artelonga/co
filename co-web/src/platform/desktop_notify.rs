//! CO-327: macOS desktop notifications.
//!
//! `send` fires a native notification on macOS (via `terminal-notifier` if
//! installed, otherwise via `osascript`) and is a no-op on all other platforms.
//! Debouncing lives in the caller (see the notification listener in
//! `server::mod::start_server_inner`).

/// Send a desktop notification with an optional click-to-open URL.
///
/// - macOS: uses `terminal-notifier` (click opens URL) when installed,
///   otherwise falls back to `osascript` (URL visible in body text).
/// - Linux / Windows: no-op.
pub fn send(title: &str, body: &str, open_url: &str) {
    #[cfg(target_os = "macos")]
    send_macos(title, body, open_url);

    // Suppress unused-variable warnings on non-macOS builds.
    #[cfg(not(target_os = "macos"))]
    let _ = (title, body, open_url);
}

/// Format a notification title and body for a given event kind.
///
/// Returns `(title, body)` ready to pass to `send`.
pub fn format_notification(kind: &str, open_url: &str) -> (String, String) {
    let description = match kind {
        "chat.dm" => "Nova mensagem direta",
        "chat.message" => "Nova mensagem",
        "chat.mention" => "Você foi mencionado",
        "universe.invitation" => "Convite para universo",
        _ => "Nova notificação",
    };
    ("CO".to_string(), format!("{description} — {open_url}"))
}

// ---------------------------------------------------------------------------
// macOS implementation
// ---------------------------------------------------------------------------

/// Escape a string for embedding inside an AppleScript string literal.
#[cfg(target_os = "macos")]
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "macos")]
fn send_macos(title: &str, body: &str, open_url: &str) {
    // Prefer terminal-notifier when installed — it supports click-to-open via
    // the -open flag.  Spawn is non-blocking; we don't wait for it to finish.
    let used_notifier = std::process::Command::new("terminal-notifier")
        .arg("-title")
        .arg(title)
        .arg("-message")
        .arg(body)
        .arg("-open")
        .arg(open_url)
        .spawn()
        .is_ok();

    if !used_notifier {
        // osascript fallback: no click handler, so include the URL in the body.
        let safe_title = escape_applescript(title);
        let safe_body = escape_applescript(&format!("{body}  {open_url}"));
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            safe_body, safe_title
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .spawn();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_does_not_panic() {
        // Smoke test: send must never panic regardless of platform or notifier state.
        send(
            "CO",
            "Test notification",
            "http://127.0.0.1:54321/notifications",
        );
    }

    #[test]
    fn format_notification_dm() {
        let (title, body) = format_notification("chat.dm", "http://127.0.0.1:8080/notifications");
        assert_eq!(title, "CO");
        assert!(body.contains("mensagem"));
        assert!(body.contains("http://127.0.0.1:8080/notifications"));
    }

    #[test]
    fn format_notification_unknown_kind() {
        let (title, body) =
            format_notification("some.unknown.event", "http://localhost/notifications");
        assert_eq!(title, "CO");
        assert!(body.contains("Nova notificação"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_applescript_quotes_and_backslashes() {
        assert_eq!(escape_applescript(r#"say "hello""#), r#"say \"hello\""#);
        assert_eq!(escape_applescript(r"back\slash"), r"back\\slash");
    }
}
