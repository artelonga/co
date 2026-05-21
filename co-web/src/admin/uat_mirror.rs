//! CO-82: UAT mirrors prod content on reset.
//!
//! When UAT_MIRROR_PROD=true and the reset flag was just processed, this module
//! pulls the content of yuri's prod universes via HTTP and writes it locally
//! through the regular Vault API. Failure modes degrade gracefully: prod down or
//! token expired = log error, UAT keeps the empty placeholders from `seed_*`.

use anyhow::{Context, Result};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};

const ALLOWED_VISIBILITIES: &[&str] = &["private", "public-subscribable", "requires_login"];

/// Reserved chars for URL path segments. Mirrors `path_segment_encode_set` from
/// the `url` crate without pulling it in as a direct dep.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

#[derive(Debug, Deserialize, Serialize)]
struct UniverseInfo {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    owner_id: String,
}

#[derive(Debug, Deserialize)]
struct VaultListEntry {
    path: String,
}

/// Default universe keys to mirror when `UAT_MIRROR_UNIVERSES` env var is unset.
const DEFAULT_UNIVERSES: &[&str] = &["artelonga", "quilomboaraucaria", "rfq"];

/// Mirror prod universes to UAT. Idempotent — safe to retry.
///
/// The set of universes to mirror is read from `UAT_MIRROR_UNIVERSES`
/// (comma-separated keys). Default: `artelonga,quilomboaraucaria,rfq`.
///
/// Discovery via `GET /api/v1/universes` was tried earlier but that endpoint
/// requires JWT auth (not API tokens). Per-universe metadata (`GET
/// /api/v1/universes/:slug`) is public, and the vault routes accept API
/// tokens, so we iterate a configured list and skip any that 404 on prod.
pub async fn mirror_prod_to_uat(prod_url: &str, prod_token: &str, local_url: &str) -> Result<()> {
    tracing::info!("UAT mirror: starting (prod={prod_url}, local={local_url})");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build http client")?;

    // 1. Login to local UAT as yuri to get a write-capable session cookie.
    let local_session = uat_login(&client, local_url).await.context("uat-login")?;

    // 2. Resolve the set of universes to mirror.
    let universe_keys: Vec<String> = std::env::var("UAT_MIRROR_UNIVERSES")
        .ok()
        .and_then(|s| {
            let v: Vec<String> = s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if v.is_empty() { None } else { Some(v) }
        })
        .unwrap_or_else(|| DEFAULT_UNIVERSES.iter().map(|s| s.to_string()).collect());
    tracing::info!(
        "UAT mirror: configured to mirror {} universe(s): {:?}",
        universe_keys.len(),
        universe_keys
    );

    // 3. Mirror each one. Per-universe failures are logged and skipped.
    for key in universe_keys {
        let info = match fetch_prod_universe(&client, prod_url, &key).await {
            Ok(Some(info)) => info,
            Ok(None) => {
                tracing::warn!("UAT mirror: '{key}' not found on prod (skipping)");
                continue;
            }
            Err(e) => {
                tracing::error!("UAT mirror: failed to fetch '{key}' from prod: {e:#}");
                continue;
            }
        };
        if let Err(e) = mirror_one_universe(
            &client,
            prod_url,
            prod_token,
            local_url,
            &local_session,
            &info,
        )
        .await
        {
            tracing::error!("UAT mirror: '{}' failed: {e:#}", info.key);
        }
    }

    tracing::info!("UAT mirror: done");
    Ok(())
}

/// Fetch metadata for a single prod universe via the public endpoint.
/// Returns `Ok(None)` if the universe doesn't exist (404), `Err` for other
/// failures.
async fn fetch_prod_universe(
    client: &reqwest::Client,
    prod_url: &str,
    key: &str,
) -> Result<Option<UniverseInfo>> {
    let resp = client
        .get(format!("{prod_url}/api/v1/universes/{key}"))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let info: UniverseInfo = resp.error_for_status()?.json().await?;
    Ok(Some(info))
}

async fn uat_login(client: &reqwest::Client, local_url: &str) -> Result<String> {
    let resp = client
        .post(format!("{local_url}/api/v1/auth/uat-login"))
        .json(&serde_json::json!({ "email": "yuri@uat.local", "password": "uat" }))
        .send()
        .await?
        .error_for_status()?;
    let session = resp
        .cookies()
        .find(|c| c.name() == "session")
        .map(|c| c.value().to_string())
        .ok_or_else(|| anyhow::anyhow!("no session cookie in uat-login response"))?;
    Ok(session)
}

async fn mirror_one_universe(
    client: &reqwest::Client,
    prod_url: &str,
    prod_token: &str,
    local_url: &str,
    local_session: &str,
    u: &UniverseInfo,
) -> Result<()> {
    // Skip system-managed universes that have their own seed paths.
    // co-dev and co-experience removed — they no longer exist (CO-142 Phase C).
    if matches!(u.key.as_str(), "template" | "yggdrasil" | "dados") {
        tracing::debug!("UAT mirror: skipping system universe '{}'", u.key);
        return Ok(());
    }

    // Create the universe locally; ignore conflict (already exists).
    let create = client
        .post(format!("{local_url}/api/v1/universes"))
        .header("cookie", format!("session={local_session}"))
        .json(&serde_json::json!({
            "key": u.key,
            "name": u.name,
            "description": u.description,
        }))
        .send()
        .await?;
    let created = create.status().is_success();
    let already = matches!(create.status().as_u16(), 409 | 500);
    if !created && !already {
        anyhow::bail!("local create returned {}", create.status());
    }

    // Best-effort: PUT visibility to match prod (only works if local owner == caller).
    if ALLOWED_VISIBILITIES.contains(&u.visibility.as_str()) {
        let _ = client
            .put(format!("{local_url}/api/v1/universes/{}", u.key))
            .header("cookie", format!("session={local_session}"))
            .json(&serde_json::json!({ "visibility": u.visibility }))
            .send()
            .await;
        // 403 here is expected for system-owned universes (quilomboaraucaria); content still flows.
    }

    // List + copy entries.
    let entries: Vec<VaultListEntry> = client
        .get(format!("{prod_url}/api/v1/universes/{}/vault/", u.key))
        .bearer_auth(prod_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Throttle: prod's vault auth (CO-35) rate-limits at 60 req/min per token.
    // Each entry copy = 2 prod requests (GET) + 1 local PUT. Pace at one
    // entry/second to keep the prod-side budget at ~30 req/min — plenty of
    // headroom and well below the cap.
    let throttle = std::time::Duration::from_millis(1000);

    let mut ok = 0usize;
    let mut fail = 0usize;
    for entry in &entries {
        tokio::time::sleep(throttle).await;
        match copy_entry(
            client,
            prod_url,
            prod_token,
            local_url,
            local_session,
            &u.key,
            &entry.path,
        )
        .await
        {
            Ok(()) => ok += 1,
            Err(e) => {
                fail += 1;
                tracing::warn!("UAT mirror: {}/{} failed: {e:#}", u.key, entry.path);
            }
        }
    }
    tracing::info!(
        "UAT mirror: {} -> {ok} ok, {fail} fail (of {})",
        u.key,
        entries.len()
    );
    Ok(())
}

async fn copy_entry(
    client: &reqwest::Client,
    prod_url: &str,
    prod_token: &str,
    local_url: &str,
    local_session: &str,
    universe_key: &str,
    path: &str,
) -> Result<()> {
    let encoded_path = path
        .split('/')
        .map(|seg| utf8_percent_encode(seg, PATH_SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/");

    // Vault GET returns JSON with a `content` field; we want the raw markdown to round-trip.
    #[derive(Deserialize)]
    struct VaultGet {
        content: String,
    }

    let body: VaultGet = client
        .get(format!(
            "{prod_url}/api/v1/universes/{universe_key}/vault/{encoded_path}"
        ))
        .bearer_auth(prod_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    client
        .put(format!(
            "{local_url}/api/v1/universes/{universe_key}/vault/{encoded_path}"
        ))
        .header("cookie", format!("session={local_session}"))
        .header("content-type", "text/markdown")
        .body(body.content)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
