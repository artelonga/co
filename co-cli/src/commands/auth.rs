//! `co auth` — password reset and API token lifecycle CLI.
//!
//! Wraps the four-step forgot-password flow and 90-day API token management
//! into typed, safe subcommands. Passwords are never echoed or stored in
//! shell history. The credentials file is created with mode 600.
//!
//! Endpoints covered (CO-165 + CO-35):
//!   POST /api/v1/auth/password-login
//!   POST /api/v1/auth/forgot-password
//!   POST /api/v1/auth/forgot-password/verify
//!   POST /api/v1/auth/reset-password
//!   POST /api/v1/auth/change-password
//!   GET  /api/v1/auth/me
//!   POST /api/v1/auth/token
//!   GET  /api/v1/auth/tokens
//!   DELETE /api/v1/auth/tokens/{id}

use std::collections::HashMap;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use colored::Colorize;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "https://co.artelonga.com.br";
const USER_AGENT: &str = concat!("co-cli/", env!("CARGO_PKG_VERSION"));

// ─── Credential types ─────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct Profile {
    /// CO server base URL for this profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Short-lived JWT session token (7 days). Used for management operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// JWT session expiry (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_expires_at: Option<String>,
    /// Long-lived API token (90 days). Used for scripting and automation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// API token ID for display and revocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    /// API token expiry (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Cached user ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Cached email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Cached display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Credentials file: a TOML map where each key is a profile name.
pub type Credentials = HashMap<String, Profile>;

// ─── Auth actions ─────────────────────────────────────────────────────────────

pub enum AuthAction {
    Login {
        email: Option<String>,
        /// Use password login instead of the default emailed magic-code.
        password: bool,
        save_token: bool,
    },
    ResetPassword {
        email: Option<String>,
    },
    ChangePassword,
    Status,
    TokenCreate {
        name: Option<String>,
        save: bool,
    },
    TokenList {
        json: bool,
    },
    TokenRevoke {
        id: String,
    },
    Logout {
        revoke_token: bool,
    },
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run(action: AuthAction, profile_name: Option<String>) {
    let profile = profile_name.as_deref().unwrap_or("default");
    match action {
        AuthAction::Login {
            email,
            password,
            save_token,
        } => cmd_login(email, password, save_token, profile),
        AuthAction::ResetPassword { email } => cmd_reset_password(email, profile),
        AuthAction::ChangePassword => cmd_change_password(profile),
        AuthAction::Status => cmd_status(profile),
        AuthAction::TokenCreate { name, save } => cmd_token_create(name, save, profile),
        AuthAction::TokenList { json } => cmd_token_list(json, profile),
        AuthAction::TokenRevoke { id } => cmd_token_revoke(id, profile),
        AuthAction::Logout { revoke_token } => cmd_logout(revoke_token, profile),
    }
}

// ─── Credential file management ──────────────────────────────────────────────

pub fn credentials_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("co")
        .join("credentials")
}

pub fn load_credentials() -> Credentials {
    let path = credentials_path();
    check_credentials_permissions(&path);
    load_credentials_from_path(&path).unwrap_or_default()
}

pub fn load_credentials_from_path(path: &Path) -> Result<Credentials> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

pub fn save_credentials(creds: &Credentials) -> Result<()> {
    save_credentials_to_path(&credentials_path(), creds)
}

pub fn save_credentials_to_path(path: &Path, creds: &Credentials) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    let raw = toml::to_string_pretty(creds).context("serializing credentials")?;
    std::fs::write(path, raw.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
    // Enforce owner-only read/write. Done after write since create mode
    // bits are advisory on some platforms.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }
    Ok(())
}

/// Warn and auto-fix credentials file if it is world- or group-readable.
pub fn check_credentials_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                eprintln!(
                    "{} {} is world/group-readable (mode {:o}). Fixing to 600...",
                    "warning:".yellow().bold(),
                    path.display(),
                    mode
                );
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

pub fn is_session_expired(profile: &Profile) -> bool {
    expired_at(profile.session_expires_at.as_deref())
}

pub fn is_token_expired(profile: &Profile) -> bool {
    profile.token.is_none() || expired_at(profile.expires_at.as_deref())
}

fn expired_at(s: Option<&str>) -> bool {
    let Some(s) = s else { return true };
    s.parse::<DateTime<Utc>>()
        .map(|exp| Utc::now() >= exp)
        .unwrap_or(true)
}

fn base_url_for(creds: &Credentials, profile_name: &str) -> String {
    creds
        .get(profile_name)
        .and_then(|p| p.base_url.clone())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// Resolve `(base_url, bearer)` for a profile, preferring a valid API token and
/// falling back to a still-valid session JWT. Used by `co sync` (CO-51) to
/// authenticate Vault API calls. Errors when neither credential is usable.
pub fn resolve_auth(profile_name: &str) -> Result<(String, String)> {
    let creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);
    let profile = creds.get(profile_name);
    let bearer = profile
        .and_then(|p| {
            if !is_token_expired(p) {
                p.token.clone()
            } else if p.session.is_some() && !is_session_expired(p) {
                p.session.clone()
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'co login' first."))?;
    Ok((base_url, bearer))
}

// ─── Token name ───────────────────────────────────────────────────────────────

pub fn default_token_name() -> String {
    let host = whoami::fallible::hostname().unwrap_or_else(|_| "local".to_string());
    let date = Utc::now().format("%Y-%m-%d");
    format!("cli-{host}-{date}")
}

// ─── HTTP helpers ─────────────────────────────────────────────────────────────

struct ApiClient {
    base_url: String,
    client: reqwest::blocking::Client,
    bearer: Option<String>,
}

impl ApiClient {
    fn new(base_url: &str, bearer: Option<&str>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            // CO-411: prod's abuse heuristics (CO-278-B/CO-397) reject
            // UA-less requests with 400 missing_user_agent — every CLI
            // client must identify itself.
            .user_agent(USER_AGENT)
            .build()
            .expect("HTTP client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
            bearer: bearer.map(str::to_string),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn post_raw(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::blocking::Response> {
        let mut req = self.client.post(self.url(path)).json(body);
        if let Some(t) = &self.bearer {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        req.send().with_context(|| format!("POST {path}"))
    }

    fn post<R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<R> {
        let resp = self.post_raw(path, body)?;
        parse_response(resp, path)
    }

    fn get<R: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<R> {
        let mut req = self.client.get(self.url(path));
        if let Some(t) = &self.bearer {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req.send().with_context(|| format!("GET {path}"))?;
        parse_response(resp, path)
    }

    fn delete(&self, path: &str) -> Result<()> {
        let mut req = self.client.delete(self.url(path));
        if let Some(t) = &self.bearer {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req.send().with_context(|| format!("DELETE {path}"))?;
        let status = resp.status();
        if !status.is_success() {
            let msg = error_message(resp);
            bail!("{msg}");
        }
        Ok(())
    }
}

fn parse_response<R: for<'de> Deserialize<'de>>(
    resp: reqwest::blocking::Response,
    path: &str,
) -> Result<R> {
    let status = resp.status();
    if !status.is_success() {
        let msg = error_message(resp);
        bail!("{msg}");
    }
    resp.json::<R>()
        .with_context(|| format!("parsing response from {path}"))
}

fn error_message(resp: reqwest::blocking::Response) -> String {
    let text = resp.text().unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("error")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or(text)
}

/// Extract the session JWT from a `Set-Cookie: session=<jwt>; ...` header.
fn extract_session_jwt(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|cookie_str| {
            cookie_str.split(';').find_map(|part| {
                let part = part.trim();
                part.strip_prefix("session=")
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            })
        })
}

// ─── I/O helpers ──────────────────────────────────────────────────────────────

fn prompt(label: &str) -> String {
    print!("{}: ", label.bold());
    io::stdout().flush().ok();
    let mut s = String::new();
    io::stdin().read_line(&mut s).ok();
    s.trim().to_string()
}

fn prompt_password(label: &str) -> String {
    rpassword::prompt_password(format!("{}: ", label.bold()))
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn prompt_password_twice(label: &str) -> Result<String> {
    loop {
        let pw1 = prompt_password(label);
        let pw2 = prompt_password(&format!("Confirm {}", label.to_lowercase()));
        if pw1.is_empty() {
            bail!("Password cannot be empty");
        }
        if pw1 == pw2 {
            return Ok(pw1);
        }
        eprintln!("{} Passwords do not match. Try again.", "!".yellow());
    }
}

fn confirm(msg: &str) -> bool {
    let answer = prompt(&format!("{} [y/N]", msg));
    matches!(answer.to_lowercase().as_str(), "y" | "yes")
}

// ─── Subcommand: login ────────────────────────────────────────────────────────

fn cmd_login(email: Option<String>, password: bool, save_token: bool, profile_name: &str) {
    if let Err(e) = do_login(email, password, save_token, profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_login(
    email: Option<String>,
    use_password: bool,
    save_token: bool,
    profile_name: &str,
) -> Result<()> {
    let mut creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);

    let email = email.unwrap_or_else(|| prompt("Email"));
    let client = ApiClient::new(&base_url, None);

    // Default: passwordless email magic-code (works for any user, including fresh
    // signups). `--password` opts into the classic password login.
    let resp = if use_password {
        let password = prompt_password("Password");
        client.post_raw(
            "/api/v1/auth/password-login",
            &serde_json::json!({ "email": email, "password": password }),
        )?
    } else {
        client.post_raw("/api/v1/auth/login", &serde_json::json!({ "email": email }))?;
        println!(
            "{} A 6-digit code was sent to {}. Check your email.",
            "→".cyan(),
            email.cyan()
        );
        let code = prompt("Code");
        client.post_raw(
            "/api/v1/auth/verify",
            &serde_json::json!({ "email": email, "code": code.trim() }),
        )?
    };

    let status = resp.status();
    let session_jwt = extract_session_jwt(resp.headers());
    let body: serde_json::Value = resp.json().unwrap_or_default();

    if !status.is_success() {
        let msg = body
            .get("error")
            .or_else(|| body.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Authentication failed");
        bail!("{msg}");
    }

    let user_id = body["user_id"].as_str().unwrap_or("").to_string();
    let user_email = body["email"].as_str().unwrap_or("").to_string();
    let display_name = body["display_name"].as_str().unwrap_or("").to_string();
    let session_expires_at = body["expires_at"].as_str().unwrap_or("").to_string();

    println!(
        "{} Logged in as {} ({})",
        "✓".green().bold(),
        display_name.cyan(),
        user_email
    );

    let profile = creds.entry(profile_name.to_string()).or_default();
    profile.base_url = Some(base_url.clone());
    profile.session = session_jwt.clone();
    profile.session_expires_at = Some(session_expires_at);
    profile.user_id = Some(user_id.clone());
    profile.email = Some(user_email.clone());
    profile.display_name = Some(display_name.clone());

    if save_token {
        let jwt = session_jwt.ok_or_else(|| {
            anyhow::anyhow!("Server did not return a session cookie — cannot create API token")
        })?;
        let token_client = ApiClient::new(&base_url, Some(&jwt));
        let token_name = default_token_name();
        let tok: serde_json::Value = token_client.post(
            "/api/v1/auth/token",
            &serde_json::json!({ "name": token_name }),
        )?;
        let raw_token = tok["token"].as_str().unwrap_or("").to_string();
        let token_id = tok["id"].as_str().unwrap_or("").to_string();
        let expires_at = tok["expires_at"].as_str().unwrap_or("").to_string();

        println!(
            "{} API token created: {} (expires {})",
            "✓".green().bold(),
            token_id.cyan(),
            expires_at
        );
        println!("{}", "Token (shown once — save it now):".yellow().bold());
        println!("{}", raw_token.green());

        let profile = creds.entry(profile_name.to_string()).or_default();
        profile.token = Some(raw_token);
        profile.token_id = Some(token_id);
        profile.expires_at = Some(expires_at);
    }

    save_credentials(&creds)?;
    println!(
        "{} Credentials saved to {}",
        "✓".green().bold(),
        credentials_path().display()
    );
    Ok(())
}

// ─── Subcommand: reset-password ───────────────────────────────────────────────

fn cmd_reset_password(email: Option<String>, profile_name: &str) {
    if let Err(e) = do_reset_password(email, profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_reset_password(email: Option<String>, profile_name: &str) -> Result<()> {
    let mut creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);
    let client = ApiClient::new(&base_url, None);

    let email = email.unwrap_or_else(|| prompt("Email or username"));

    // Step 1: request reset code
    let sent: serde_json::Value = client.post(
        "/api/v1/auth/forgot-password",
        &serde_json::json!({
            "username_or_channel_value": email,
            "email": email
        }),
    )?;
    let channels = sent["sent_to"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if channels.is_empty() {
        println!(
            "{} Code sent (channel details hidden for privacy).",
            "→".blue()
        );
    } else {
        println!("{} Code sent to: {}", "→".blue(), channels.cyan());
    }

    // Step 2: verify code
    let code = prompt("Enter code");
    let verified: serde_json::Value = client.post(
        "/api/v1/auth/forgot-password/verify",
        &serde_json::json!({
            "username_or_channel_value": email,
            "email": email,
            "code": code
        }),
    )?;
    let reset_token = verified["reset_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Server did not return a reset token"))?
        .to_string();

    // Step 3: new password
    let new_password = prompt_password_twice("New password")?;

    // Step 4: apply reset
    client.post::<serde_json::Value>(
        "/api/v1/auth/reset-password",
        &serde_json::json!({
            "reset_token": reset_token,
            "new_password": new_password
        }),
    )?;
    println!("{} Password reset successfully.", "✓".green().bold());

    // Step 5: auto-login
    println!("{}", "Signing in with new password...".dimmed());
    let login_resp = client.post_raw(
        "/api/v1/auth/password-login",
        &serde_json::json!({ "email": email, "password": new_password }),
    )?;
    let login_status = login_resp.status();
    let session_jwt = extract_session_jwt(login_resp.headers());
    let login_body: serde_json::Value = login_resp.json().unwrap_or_default();

    if !login_status.is_success() {
        eprintln!(
            "{} Auto-login failed — run 'co auth login' manually.",
            "warning:".yellow().bold()
        );
        return Ok(());
    }

    let user_id = login_body["user_id"].as_str().unwrap_or("").to_string();
    let user_email = login_body["email"].as_str().unwrap_or("").to_string();
    let display_name = login_body["display_name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let session_expires_at = login_body["expires_at"].as_str().unwrap_or("").to_string();
    println!(
        "{} Signed in as {} ({})",
        "✓".green().bold(),
        display_name.cyan(),
        user_email
    );

    let profile = creds.entry(profile_name.to_string()).or_default();
    profile.base_url = Some(base_url.clone());
    profile.session = session_jwt.clone();
    profile.session_expires_at = Some(session_expires_at);
    profile.user_id = Some(user_id);
    profile.email = Some(user_email);
    profile.display_name = Some(display_name);

    // Step 6: optionally create and save API token
    if confirm("Create and save a 90-day API token?") {
        if let Some(jwt) = session_jwt {
            let token_client = ApiClient::new(&base_url, Some(&jwt));
            let tok: serde_json::Value = token_client.post(
                "/api/v1/auth/token",
                &serde_json::json!({ "name": default_token_name() }),
            )?;
            let raw_token = tok["token"].as_str().unwrap_or("").to_string();
            let token_id = tok["id"].as_str().unwrap_or("").to_string();
            let expires_at = tok["expires_at"].as_str().unwrap_or("").to_string();
            println!("{}", "Token (shown once — save it now):".yellow().bold());
            println!("{}", raw_token.green());
            let profile = creds.entry(profile_name.to_string()).or_default();
            profile.token = Some(raw_token);
            profile.token_id = Some(token_id);
            profile.expires_at = Some(expires_at);
        } else {
            eprintln!(
                "{} No session cookie — cannot create token.",
                "warning:".yellow().bold()
            );
        }
    }

    save_credentials(&creds)?;
    println!(
        "{} Credentials saved to {}",
        "✓".green().bold(),
        credentials_path().display()
    );
    Ok(())
}

// ─── Subcommand: change-password ─────────────────────────────────────────────

fn cmd_change_password(profile_name: &str) {
    if let Err(e) = do_change_password(profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_change_password(profile_name: &str) -> Result<()> {
    let creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);

    let profile = creds.get(profile_name);
    let jwt = profile
        .and_then(|p| p.session.as_deref())
        .filter(|_| !is_session_expired(profile.unwrap()))
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'co auth login' first."))?;

    let current = prompt_password("Current password");
    let new_password = prompt_password_twice("New password")?;

    let client = ApiClient::new(&base_url, Some(jwt));
    client.post::<serde_json::Value>(
        "/api/v1/auth/change-password",
        &serde_json::json!({
            "current_password": current,
            "new_password": new_password
        }),
    )?;
    println!("{} Password changed.", "✓".green().bold());
    Ok(())
}

// ─── Subcommand: status ───────────────────────────────────────────────────────

fn cmd_status(profile_name: &str) {
    let creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);
    let profile = creds.get(profile_name);

    let has_session = profile
        .map(|p| p.session.is_some() && !is_session_expired(p))
        .unwrap_or(false);
    let has_token = profile.map(|p| !is_token_expired(p)).unwrap_or(false);

    if !has_session && !has_token {
        println!("{} Not authenticated.", "✗".red().bold());
        println!("Run 'co auth login' to sign in.");
        std::process::exit(1);
    }

    println!("{}", "CO Auth Status".bold().green());
    println!("{}", "─".repeat(40));
    println!("  Profile:  {}", profile_name.cyan());
    println!("  Base URL: {}", base_url.cyan());

    if let Some(p) = profile {
        if let Some(ref name) = p.display_name {
            println!("  User:     {}", name.cyan());
        }
        if let Some(ref email) = p.email {
            println!("  Email:    {}", email.cyan());
        }

        if has_session {
            let exp = p.session_expires_at.as_deref().unwrap_or("unknown");
            println!("  Session:  {} (expires {})", "valid".green(), exp);
        } else {
            println!("  Session:  {}", "expired".yellow());
        }

        if p.token.is_some() {
            if has_token {
                let exp = p.expires_at.as_deref().unwrap_or("unknown");
                let prefix = p
                    .token
                    .as_deref()
                    .unwrap_or("")
                    .get(..11)
                    .unwrap_or("co_...");
                println!(
                    "  API token: {} ({}, expires {})",
                    prefix.cyan(),
                    "valid".green(),
                    exp
                );
                if let Some(ref id) = p.token_id {
                    println!("  Token ID:  {}", id.dimmed());
                }
            } else {
                println!("  API token: {}", "expired".yellow());
            }
        }
    }

    // Verify session live against server (best-effort, JWT-only endpoint)
    if has_session {
        let jwt = profile.and_then(|p| p.session.as_deref()).unwrap_or("");
        let client = ApiClient::new(&base_url, Some(jwt));
        match client.get::<serde_json::Value>("/api/v1/auth/me") {
            Ok(me) => {
                let tier = me["tier"].as_str().unwrap_or("unknown");
                println!("  Server:   {} (tier: {})", "verified".green(), tier);
            }
            Err(_) => {
                println!(
                    "  Server:   {} (could not reach {})",
                    "unverified".yellow(),
                    base_url
                );
            }
        }
    }
}

// ─── Subcommand: token create ─────────────────────────────────────────────────

fn cmd_token_create(name: Option<String>, save: bool, profile_name: &str) {
    if let Err(e) = do_token_create(name, save, profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_token_create(name: Option<String>, save: bool, profile_name: &str) -> Result<()> {
    let mut creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);

    let profile = creds.get(profile_name);
    let jwt = profile
        .and_then(|p| p.session.as_deref())
        .filter(|_| !is_session_expired(profile.unwrap()))
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'co auth login' first."))?
        .to_string();

    let token_name = name.unwrap_or_else(default_token_name);
    let client = ApiClient::new(&base_url, Some(&jwt));
    let tok: serde_json::Value = client.post(
        "/api/v1/auth/token",
        &serde_json::json!({ "name": token_name }),
    )?;

    let raw_token = tok["token"].as_str().unwrap_or("").to_string();
    let token_id = tok["id"].as_str().unwrap_or("").to_string();
    let tok_name = tok["name"].as_str().unwrap_or("").to_string();
    let expires_at = tok["expires_at"].as_str().unwrap_or("").to_string();

    println!(
        "{} Token created: {} \"{}\" (expires {})",
        "✓".green().bold(),
        token_id.cyan(),
        tok_name,
        expires_at
    );
    println!(
        "{}",
        "Token value (shown once — save it now):".yellow().bold()
    );
    println!("{}", raw_token.green());

    if save {
        let profile = creds.entry(profile_name.to_string()).or_default();
        profile.token = Some(raw_token);
        profile.token_id = Some(token_id);
        profile.expires_at = Some(expires_at);
        save_credentials(&creds)?;
        println!(
            "{} Token saved to {}",
            "✓".green().bold(),
            credentials_path().display()
        );
    }
    Ok(())
}

// ─── Subcommand: token list ───────────────────────────────────────────────────

fn cmd_token_list(json: bool, profile_name: &str) {
    if let Err(e) = do_token_list(json, profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_token_list(json: bool, profile_name: &str) -> Result<()> {
    let creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);

    let profile = creds.get(profile_name);
    let jwt = profile
        .and_then(|p| p.session.as_deref())
        .filter(|_| !is_session_expired(profile.unwrap()))
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'co auth login' first."))?;

    let client = ApiClient::new(&base_url, Some(jwt));
    let tokens: Vec<serde_json::Value> = client.get("/api/v1/auth/tokens")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&tokens)?);
        return Ok(());
    }

    if tokens.is_empty() {
        println!("No API tokens.");
        return Ok(());
    }

    use comfy_table::{Table, presets::UTF8_FULL};
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(["ID", "Name", "Prefix", "Expires", "Last used"]);
    for tok in &tokens {
        table.add_row([
            tok["id"].as_str().unwrap_or("-"),
            tok["name"].as_str().unwrap_or("-"),
            tok["token_prefix"].as_str().unwrap_or("-"),
            tok["expires_at"].as_str().unwrap_or("-"),
            tok["last_used_at"].as_str().unwrap_or("never"),
        ]);
    }
    println!("{table}");
    Ok(())
}

// ─── Subcommand: token revoke ─────────────────────────────────────────────────

fn cmd_token_revoke(id: String, profile_name: &str) {
    if let Err(e) = do_token_revoke(id, profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_token_revoke(id: String, profile_name: &str) -> Result<()> {
    let mut creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);

    let profile = creds.get(profile_name);
    let jwt = profile
        .and_then(|p| p.session.as_deref())
        .filter(|_| !is_session_expired(profile.unwrap()))
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'co auth login' first."))?
        .to_string();

    if !confirm(&format!("Revoke token {}?", id.cyan())) {
        println!("Aborted.");
        return Ok(());
    }

    let client = ApiClient::new(&base_url, Some(&jwt));
    client.delete(&format!("/api/v1/auth/tokens/{id}"))?;
    println!("{} Token {} revoked.", "✓".green().bold(), id.cyan());

    // If this was the stored token, clear it
    if let Some(profile) = creds.get_mut(profile_name)
        && profile.token_id.as_deref() == Some(&id)
    {
        profile.token = None;
        profile.token_id = None;
        profile.expires_at = None;
        save_credentials(&creds)?;
    }
    Ok(())
}

// ─── Subcommand: logout ───────────────────────────────────────────────────────

fn cmd_logout(revoke_token: bool, profile_name: &str) {
    if let Err(e) = do_logout(revoke_token, profile_name) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_logout(revoke_token: bool, profile_name: &str) -> Result<()> {
    let mut creds = load_credentials();
    let base_url = base_url_for(&creds, profile_name);

    if revoke_token {
        let profile = creds.get(profile_name);
        let jwt = profile.and_then(|p| p.session.as_deref());
        let token_id = profile
            .and_then(|p| p.token_id.as_deref())
            .map(str::to_string);

        if let (Some(jwt), Some(id)) = (jwt, token_id) {
            if !is_session_expired(profile.unwrap()) {
                let client = ApiClient::new(&base_url, Some(jwt));
                match client.delete(&format!("/api/v1/auth/tokens/{id}")) {
                    Ok(()) => println!("{} API token revoked.", "✓".green().bold()),
                    Err(e) => {
                        eprintln!("{} Could not revoke token: {e}", "warning:".yellow().bold())
                    }
                }
            } else {
                eprintln!(
                    "{} Session expired — cannot revoke token server-side.",
                    "warning:".yellow().bold()
                );
            }
        } else {
            eprintln!("{} No stored token to revoke.", "warning:".yellow().bold());
        }
    }

    if let Some(profile) = creds.get_mut(profile_name) {
        profile.session = None;
        profile.session_expires_at = None;
        profile.token = None;
        profile.token_id = None;
        profile.expires_at = None;
    }
    save_credentials(&creds)?;
    println!(
        "{} Logged out of profile '{}'.",
        "✓".green().bold(),
        profile_name
    );
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_token_name_format() {
        let name = default_token_name();
        assert!(name.starts_with("cli-"), "must start with 'cli-': {name}");
        // format: cli-<hostname>-<YYYY-MM-DD>
        let parts: Vec<&str> = name.splitn(3, '-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "cli");
        assert!(!parts[1].is_empty(), "hostname part must not be empty");
        assert!(
            parts[2].len() >= 8,
            "date part must be at least YYYY-MM-DD: {}",
            parts[2]
        );
    }

    #[test]
    fn test_credentials_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("credentials");

        let mut creds = Credentials::new();
        creds.insert(
            "default".to_string(),
            Profile {
                base_url: Some("https://example.com".to_string()),
                token: Some("co_abc123".to_string()),
                token_id: Some("tok_xyz".to_string()),
                user_id: Some("usr_123".to_string()),
                email: Some("test@example.com".to_string()),
                display_name: Some("Test User".to_string()),
                expires_at: Some("2099-08-17T00:00:00Z".to_string()),
                session: None,
                session_expires_at: None,
            },
        );

        save_credentials_to_path(&path, &creds).unwrap();
        let loaded = load_credentials_from_path(&path).unwrap();

        let default = loaded.get("default").expect("default profile");
        assert_eq!(default.token.as_deref(), Some("co_abc123"));
        assert_eq!(default.user_id.as_deref(), Some("usr_123"));
        assert_eq!(default.email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn test_is_token_expired_past() {
        let profile = Profile {
            token: Some("co_tok".to_string()),
            expires_at: Some("2020-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(is_token_expired(&profile), "past date must be expired");
    }

    #[test]
    fn test_is_token_expired_future() {
        let profile = Profile {
            token: Some("co_tok".to_string()),
            expires_at: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(
            !is_token_expired(&profile),
            "future date must not be expired"
        );
    }

    #[test]
    fn test_is_token_expired_no_token() {
        let profile = Profile::default();
        assert!(is_token_expired(&profile), "missing token must be expired");
    }

    #[test]
    fn test_is_session_expired_past() {
        let profile = Profile {
            session: Some("eyJ...".to_string()),
            session_expires_at: Some("2020-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(is_session_expired(&profile));
    }

    #[test]
    fn test_is_session_not_expired_future() {
        let profile = Profile {
            session: Some("eyJ...".to_string()),
            session_expires_at: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        assert!(!is_session_expired(&profile));
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent");
        let result = load_credentials_from_path(&path);
        assert!(result.is_err(), "loading missing file should error");
    }

    #[cfg(unix)]
    #[test]
    fn test_credentials_file_mode_600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("credentials");

        let creds = Credentials::new();
        save_credentials_to_path(&path, &creds).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "credentials file must be mode 600, got {mode:o}"
        );
    }

    #[test]
    fn test_multiple_profiles_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("credentials");

        let mut creds = Credentials::new();
        creds.insert(
            "default".to_string(),
            Profile {
                base_url: Some("https://co.artelonga.com.br".to_string()),
                token: Some("co_prod_token".to_string()),
                ..Default::default()
            },
        );
        creds.insert(
            "uat".to_string(),
            Profile {
                base_url: Some("https://co-artelonga-uat.fly.dev".to_string()),
                token: Some("co_uat_token".to_string()),
                ..Default::default()
            },
        );

        save_credentials_to_path(&path, &creds).unwrap();
        let loaded = load_credentials_from_path(&path).unwrap();

        assert_eq!(
            loaded.get("default").unwrap().token.as_deref(),
            Some("co_prod_token")
        );
        assert_eq!(
            loaded.get("uat").unwrap().token.as_deref(),
            Some("co_uat_token")
        );
        assert_eq!(
            loaded.get("uat").unwrap().base_url.as_deref(),
            Some("https://co-artelonga-uat.fly.dev")
        );
    }
}

#[cfg(test)]
mod ua_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// CO-411: every ApiClient request must carry a User-Agent — prod's
    /// abuse heuristics reject UA-less requests with 400 missing_user_agent.
    #[test]
    fn test_api_client_sends_user_agent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
            req
        });
        let client = ApiClient::new(&format!("http://{addr}"), None);
        let _ = client.client.get(client.url("/probe")).send();
        let req = handle.join().unwrap();
        let ua_line = req
            .lines()
            .find(|l| l.to_lowercase().starts_with("user-agent:"))
            .unwrap_or_else(|| panic!("no User-Agent header in request:\n{req}"));
        assert!(
            ua_line.contains("co-cli/"),
            "expected co-cli/<version> UA, got: {ua_line}"
        );
    }
}
