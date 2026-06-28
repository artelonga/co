//! Canonical-tool wiring for co-web (CO-503).
//!
//! `co-web` has `reqwest`/`tokio`; the `co-engine` core crate does not. So the
//! HTTP [`ExternalInvoker`] for `url:` manifest tools lives here and is injected
//! into the dep-light core contract. This module also builds the
//! [`CanonicalToolRegistry`] at boot from the trusted manifest directory, behind
//! the opt-in `CO_ENABLE_EXTERNAL_TOOLS` flag.
//!
//! Trust boundary: see `co_engine::canon_tool` — manifests load only from the
//! instance-controlled `CO_TOOLS_DIR` (default `tools.d`), never from universe
//! content; external tools are off unless explicitly enabled.

use std::path::PathBuf;
use std::time::Duration;

use co_engine::canon_tool::{
    external_tools_enabled, register_external_tools, CanonicalToolRegistry, ExternalInvoker,
    ExternalSpec, SubprocessInvoker, ToolManifest,
};
use serde_json::Value;

/// Default request timeout for HTTP tool invocations.
const HTTP_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP [`ExternalInvoker`] for `url:` manifest tools. POSTs the JSON args to the
/// manifest's `url` and parses the JSON response.
///
/// `CanonicalTool::run` is synchronous, so this bridges to async `reqwest` by
/// running the request on a short-lived current-thread runtime inside a scoped
/// thread. That works regardless of whether the caller is already on a tokio
/// runtime (avoiding the "runtime within a runtime" panic).
pub struct HttpInvoker {
    client: reqwest::Client,
    timeout: Duration,
}

impl HttpInvoker {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout: HTTP_TOOL_TIMEOUT,
        }
    }

    /// Reuse an existing (timed) client.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            timeout: HTTP_TOOL_TIMEOUT,
        }
    }
}

impl Default for HttpInvoker {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalInvoker for HttpInvoker {
    fn invoke(&self, spec: &ExternalSpec, args: Value) -> anyhow::Result<Value> {
        let url = spec
            .url
            .clone()
            .ok_or_else(|| anyhow::anyhow!("HTTP tool has no `url`"))?;
        let client = self.client.clone();
        let timeout = self.timeout;

        // Run the async request on its own current-thread runtime in a scoped
        // thread so this works whether or not we're inside a tokio runtime.
        std::thread::scope(|s| {
            s.spawn(|| -> anyhow::Result<Value> {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async move {
                    let resp = client
                        .post(&url)
                        .json(&args)
                        .timeout(timeout)
                        .send()
                        .await
                        .map_err(|e| anyhow::anyhow!("tool service unreachable: {e}"))?;
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        anyhow::bail!("tool service returned {status}: {}", body.trim());
                    }
                    let value: Value = resp
                        .json()
                        .await
                        .map_err(|e| anyhow::anyhow!("tool response was not JSON: {e}"))?;
                    Ok(value)
                })
            })
            .join()
            .map_err(|_| anyhow::anyhow!("HTTP tool worker thread panicked"))?
        })
    }
}

/// Pick an invoker for a manifest: subprocess for `command:`, HTTP for `url:`.
pub fn invoker_for(manifest: &ToolManifest) -> anyhow::Result<Box<dyn ExternalInvoker>> {
    if manifest.command.is_some() {
        Ok(Box::new(SubprocessInvoker))
    } else if manifest.url.is_some() {
        Ok(Box::new(HttpInvoker::new()))
    } else {
        anyhow::bail!(
            "tool manifest `{}` has neither `command` nor `url`",
            manifest.name
        )
    }
}

/// The trusted manifest directory: `CO_TOOLS_DIR` or `tools.d` under the cwd.
pub fn tools_dir() -> PathBuf {
    std::env::var_os("CO_TOOLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tools.d"))
}

/// Build the canonical tool registry at boot.
///
/// External tools are loaded from the trusted manifest directory **only when**
/// `CO_ENABLE_EXTERNAL_TOOLS` is set (default OFF). When disabled, an empty
/// registry is returned (native/legacy tools can still be registered by the
/// caller through the same registry).
pub fn build_canonical_registry() -> anyhow::Result<CanonicalToolRegistry> {
    let mut registry = CanonicalToolRegistry::new();
    if external_tools_enabled() {
        let dir = tools_dir();
        let n = register_external_tools(&mut registry, &dir, invoker_for)?;
        tracing::info!(
            count = n,
            dir = %dir.display(),
            "CO-503: registered external tools from trusted manifest dir"
        );
    } else {
        tracing::debug!("CO-503: external tools disabled (CO_ENABLE_EXTERNAL_TOOLS unset)");
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use co_engine::canon_tool::CanonicalTool;
    use serde_json::json;

    #[test]
    fn http_invoker_requires_url() {
        let inv = HttpInvoker::new();
        let spec = ExternalSpec {
            command: Some("cat".into()),
            url: None,
            secrets: vec![],
        };
        let err = inv.invoke(&spec, json!({})).unwrap_err();
        assert!(err.to_string().contains("no `url`"));
    }

    #[test]
    fn invoker_for_picks_subprocess_then_http() {
        let mut m = ToolManifest {
            name: "t".into(),
            description: String::new(),
            input_schema: Value::Null,
            tool_type: Default::default(),
            command: Some("cat".into()),
            url: None,
            dependencies: vec![],
            category: None,
            status: None,
            secrets: vec![],
        };
        // command → subprocess (smoke: round-trips through real `cat`).
        let inv = invoker_for(&m).unwrap();
        let out = inv
            .invoke(
                &ExternalSpec {
                    command: Some("cat".into()),
                    url: None,
                    secrets: vec![],
                },
                json!({"a": 1}),
            )
            .unwrap();
        assert_eq!(out, json!({"a": 1}));

        // url → HTTP invoker (constructs without error; no network here).
        m.command = None;
        m.url = Some("http://localhost:1/none".into());
        assert!(invoker_for(&m).is_ok());

        // neither → error.
        m.url = None;
        assert!(invoker_for(&m).is_err());
    }

    #[test]
    fn build_registry_disabled_is_empty() {
        if std::env::var("CO_ENABLE_EXTERNAL_TOOLS").is_err() {
            let reg = build_canonical_registry().unwrap();
            assert!(reg.is_empty());
        }
    }

    #[test]
    fn build_registry_enabled_loads_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("echo.yaml"),
            "name: echo\ncommand: cat\nstatus: active\n",
        )
        .unwrap();
        // Drive the loader directly to avoid mutating process-global env in tests.
        let mut reg = CanonicalToolRegistry::new();
        let n = register_external_tools(&mut reg, dir.path(), invoker_for).unwrap();
        assert_eq!(n, 1);
        let out = reg.get("echo").unwrap().run(json!({"x": 9})).unwrap();
        assert_eq!(out, json!({"x": 9}));
    }
}
