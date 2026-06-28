//! Canonical Tool contract + external-tool adapter (CO-503).
//!
//! ## Why this exists
//!
//! CO historically had a *legacy* tool trait ([`crate::tool::Tool`], with
//! `run(action, params) -> String`) and, separately, the agent's Python
//! `CoTools` (an Anthropic-JSON-schema shape: `name` / `description` /
//! `input_schema` / `run(args)`). A tool is only useful to an agent when it
//! presents that LLM-shaped contract.
//!
//! This module defines **one canonical contract** — [`CanonicalTool`] — so that:
//!
//! * Native, in-binary tools implement the trait directly.
//! * Out-of-process tools (any OSS CLI/script, or an HTTP service such as a
//!   `yt-summarizer` or a RAG service) become tools via a **manifest entry**
//!   ([`ToolManifest`]) wrapped by [`ExternalTool`] — **with zero binary
//!   recompile**. Adding a tool is editing a YAML file, not shipping Rust.
//! * The existing legacy tools keep working through [`LegacyToolAdapter`].
//!
//! In-binary and out-of-process tools are *called identically*: an agent
//! enumerates [`CanonicalToolRegistry::list`] and invokes
//! [`CanonicalTool::run`] without knowing which one runs the work.
//!
//! ## Dependency posture
//!
//! The `core` crate has `serde`/`serde_json` but **not** `reqwest`/`tokio`.
//! So this module is dep-light: the subprocess invoker uses only `std::process`,
//! and HTTP invocation is **injectable** via the [`ExternalInvoker`] trait — the
//! concrete `reqwest` implementation lives in `co-web`.
//!
//! ## Trust boundary (SECURITY)
//!
//! External tools are **opt-in** (default OFF — see [`external_tools_enabled`],
//! gated on `CO_ENABLE_EXTERNAL_TOOLS`). Manifests are loaded **only** from a
//! trusted, instance-controlled directory ([`load_manifests_from_dir`]) — never
//! from universe/observed content, which an untrusted author could use to inject
//! a `command`. Subprocess tools run with **no ambient secrets**: a spawned
//! command inherits an empty environment unless the manifest explicitly declares
//! the variables it needs (`secrets:`), which are then read from the host env and
//! passed through by name only. This keeps a manifest-declared OSS tool from
//! exfiltrating unrelated credentials.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// ToolType
// ---------------------------------------------------------------------------

/// Whether a tool's output is reproducible (a script) or generative (LLM-backed).
///
/// Mirrors `tools/schema.yaml`'s `tool_type: deterministic | predictive`, so a
/// deterministic script-tool (CO-521) and an LLM-backed tool (CO-522) register
/// and dispatch under the same contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolType {
    /// Reproducible output for a given input (CLI/script/HTTP).
    #[default]
    Deterministic,
    /// Generative / model-backed output (an LLM tool).
    Predictive,
}

impl ToolType {
    /// Canonical lowercase string (matches the schema vocabulary).
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolType::Deterministic => "deterministic",
            ToolType::Predictive => "predictive",
        }
    }

    /// Parse from the schema vocabulary.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "deterministic" => Ok(ToolType::Deterministic),
            "predictive" => Ok(ToolType::Predictive),
            other => bail!("unknown tool_type: {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// CanonicalTool
// ---------------------------------------------------------------------------

/// The single LLM-shaped tool contract.
///
/// `name` / `description` / `input_schema` / `run(args) -> result` — the exact
/// shape both Ollama and Claude tool-calling loops consume. Implementations may
/// be in-binary (a native Rust tool) or out-of-process ([`ExternalTool`]); the
/// caller cannot tell the difference.
pub trait CanonicalTool: Send + Sync {
    /// Stable tool identifier the LLM references in a tool call.
    fn name(&self) -> &str;

    /// Human-readable description (the LLM uses this to decide when to call).
    fn description(&self) -> &str;

    /// JSON Schema describing the tool's arguments (the `input_schema` field).
    fn input_schema(&self) -> Value;

    /// Deterministic (script) vs predictive (LLM) — defaults to deterministic.
    fn tool_type(&self) -> ToolType {
        ToolType::Deterministic
    }

    /// Execute the tool with JSON arguments, returning a JSON result.
    fn run(&self, args: Value) -> Result<Value>;
}

/// The enumerable surface of a tool — what an agent lists before calling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub tool_type: ToolType,
}

// ---------------------------------------------------------------------------
// CanonicalToolRegistry
// ---------------------------------------------------------------------------

/// Registry of canonical tools an agent can enumerate and dispatch.
#[derive(Default)]
pub struct CanonicalToolRegistry {
    tools: Vec<Box<dyn CanonicalTool>>,
}

impl CanonicalToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool. A later registration with the same name shadows nothing
    /// — [`get`](Self::get) returns the first match, so callers should avoid
    /// duplicate names (manifest discovery reports duplicates as errors).
    pub fn register(&mut self, tool: Box<dyn CanonicalTool>) {
        self.tools.push(tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn CanonicalTool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| &**t)
    }

    /// True if a tool with this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name() == name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Enumerate the LLM-ready descriptors (name + description + input_schema +
    /// tool_type) for every registered tool.
    pub fn list(&self) -> Vec<ToolDescriptor> {
        self.tools
            .iter()
            .map(|t| ToolDescriptor {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
                tool_type: t.tool_type(),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// External invocation
// ---------------------------------------------------------------------------

/// How an external tool is reached — a subprocess `command` or an HTTP `url`.
///
/// Carried by [`ExternalTool`] and handed to an [`ExternalInvoker`]. Both fields
/// are optional; the invoker picks the one it understands and errors otherwise.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExternalSpec {
    /// Command line for a subprocess tool (whitespace-split; wrap complex
    /// invocations in a script). Mutually exclusive with `url` in practice.
    pub command: Option<String>,
    /// HTTP endpoint for a service tool (args POSTed as JSON, JSON read back).
    pub url: Option<String>,
    /// Names of environment variables this tool is permitted to receive. Only
    /// these are forwarded from the host env to a subprocess; everything else is
    /// stripped (no ambient secrets — see the module trust-boundary doc).
    pub secrets: Vec<String>,
}

/// Pluggable backend that actually executes an [`ExternalSpec`].
///
/// [`SubprocessInvoker`] (std-only, in this crate) handles `command` specs. An
/// HTTP invoker handling `url` specs is injected from `co-web` (which has
/// `reqwest`), so the contract supports out-of-process service tools without
/// pulling an HTTP stack into `core`.
pub trait ExternalInvoker: Send + Sync {
    fn invoke(&self, spec: &ExternalSpec, args: Value) -> Result<Value>;
}

/// Subprocess invoker: spawns `spec.command`, writes the JSON `args` to the
/// child's stdin, and parses the child's stdout as JSON. A non-zero exit maps to
/// an error (stderr included).
///
/// The child inherits **no environment** except the variables named in
/// `spec.secrets` (forwarded by name from the host env).
pub struct SubprocessInvoker;

impl ExternalInvoker for SubprocessInvoker {
    fn invoke(&self, spec: &ExternalSpec, args: Value) -> Result<Value> {
        let command = spec
            .command
            .as_deref()
            .ok_or_else(|| anyhow!("subprocess tool has no `command`"))?;

        let parts: Vec<&str> = command.split_whitespace().collect();
        let (program, prog_args) = parts
            .split_first()
            .ok_or_else(|| anyhow!("empty `command`"))?;

        let mut cmd = Command::new(program);
        cmd.args(prog_args);
        // No ambient secrets: start from a clean env, then forward only the
        // explicitly declared variables.
        cmd.env_clear();
        for key in &spec.secrets {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn tool command: {program}"))?;

        // Write args to stdin on a separate thread so a large child stdout can't
        // deadlock us against an unread stdin pipe.
        let payload = serde_json::to_vec(&args)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open child stdin"))?;
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&payload);
            // Dropping `stdin` closes the pipe, signalling EOF to the child.
        });

        let output = child.wait_with_output()?;
        let _ = writer.join();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("tool command failed ({}): {}", output.status, stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(trimmed)
            .with_context(|| format!("tool stdout was not valid JSON: {trimmed}"))
    }
}

// ---------------------------------------------------------------------------
// Manifest + ExternalTool
// ---------------------------------------------------------------------------

/// A trusted manifest entry describing an external (out-of-process) tool.
///
/// Fields map onto `tools/schema.yaml` (`category` / `tool_type` / `command` /
/// `status` / `dependencies` / `url`). A manifest with a `command` is a
/// subprocess tool; one with a `url` is a service tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema for the tool's arguments. If omitted, an empty object schema
    /// (`{"type":"object"}`) is synthesized.
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub tool_type: ToolType,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// `active` tools are registered; any other value is skipped at discovery.
    #[serde(default)]
    pub status: Option<String>,
    /// Environment variables this tool is permitted to receive (subprocess only).
    #[serde(default)]
    pub secrets: Vec<String>,
}

impl ToolManifest {
    /// The `input_schema`, defaulting to an empty object schema when unset.
    fn effective_schema(&self) -> Value {
        if self.input_schema.is_null() {
            json!({ "type": "object", "properties": {} })
        } else {
            self.input_schema.clone()
        }
    }

    fn spec(&self) -> ExternalSpec {
        ExternalSpec {
            command: self.command.clone(),
            url: self.url.clone(),
            secrets: self.secrets.clone(),
        }
    }
}

/// A [`CanonicalTool`] backed by a trusted [`ToolManifest`] and an injected
/// [`ExternalInvoker`]. This is how any OSS CLI/script or HTTP service becomes a
/// first-class CO tool without a recompile.
pub struct ExternalTool {
    manifest: ToolManifest,
    schema: Value,
    spec: ExternalSpec,
    invoker: Box<dyn ExternalInvoker>,
}

impl ExternalTool {
    pub fn new(manifest: ToolManifest, invoker: Box<dyn ExternalInvoker>) -> Self {
        let schema = manifest.effective_schema();
        let spec = manifest.spec();
        Self {
            manifest,
            schema,
            spec,
            invoker,
        }
    }
}

impl CanonicalTool for ExternalTool {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn tool_type(&self) -> ToolType {
        self.manifest.tool_type
    }

    fn run(&self, args: Value) -> Result<Value> {
        validate_required(&args, &self.schema)?;
        self.invoker.invoke(&self.spec, args)
    }
}

/// Minimal input validation: every key listed under the schema's `required`
/// array must be present in the (object) `args`.
pub fn validate_required(args: &Value, schema: &Value) -> Result<()> {
    let Some(required) = schema.get("required").and_then(|r| r.as_array()) else {
        return Ok(());
    };
    if required.is_empty() {
        return Ok(());
    }
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow!("tool args must be a JSON object"))?;
    for key in required {
        if let Some(k) = key.as_str()
            && !obj.contains_key(k)
        {
            bail!("missing required argument: {k}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Legacy bridge
// ---------------------------------------------------------------------------

/// Adapts a legacy [`crate::tool::Tool`] (with `run(action, params) -> String`)
/// to the canonical contract, so the existing github/claude/obsidian tools are
/// callable through [`CanonicalToolRegistry`] without changing the legacy trait.
///
/// The synthesized schema is `{action: string (required), params: object}`. The
/// canonical `run(args)` reads `action` + `params` and returns the legacy string
/// output wrapped as `{"output": "..."}`.
pub struct LegacyToolAdapter {
    inner: Box<dyn crate::tool::Tool>,
    schema: Value,
}

impl LegacyToolAdapter {
    pub fn new(inner: Box<dyn crate::tool::Tool>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Legacy action verb to dispatch."
                },
                "params": {
                    "type": "object",
                    "description": "String→string parameters for the action.",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["action"]
        });
        Self { inner, schema }
    }
}

impl CanonicalTool for LegacyToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn tool_type(&self) -> ToolType {
        ToolType::Deterministic
    }

    fn run(&self, args: Value) -> Result<Value> {
        validate_required(&args, &self.schema)?;
        let action = args
            .get("action")
            .and_then(|a| a.as_str())
            .ok_or_else(|| anyhow!("`action` must be a string"))?;

        let mut params: HashMap<String, String> = HashMap::new();
        if let Some(obj) = args.get("params").and_then(|p| p.as_object()) {
            for (k, v) in obj {
                // String values pass through as-is; non-strings are stringified
                // so the legacy string-map contract is preserved.
                let s = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                params.insert(k.clone(), s);
            }
        }

        let output = self.inner.run(action, &params)?;
        Ok(json!({ "output": output }))
    }
}

// ---------------------------------------------------------------------------
// Manifest discovery
// ---------------------------------------------------------------------------

/// Whether external tools are enabled (opt-in; default OFF).
///
/// Reads `CO_ENABLE_EXTERNAL_TOOLS`; truthy = `1`/`true`/`yes`/`on`
/// (case-insensitive). Any other value, or unset, returns `false`.
pub fn external_tools_enabled() -> bool {
    match std::env::var("CO_ENABLE_EXTERNAL_TOOLS") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Load all `*.yaml` / `*.yml` tool manifests from a **trusted** directory.
///
/// SECURITY: callers must pass an instance-controlled path (e.g. the repo
/// `tools.d/` or an instance config dir) — never a path derived from universe or
/// observed content. Files that fail to parse are reported as errors (a bad
/// manifest is a deployment bug, not silently ignored).
pub fn load_manifests_from_dir(dir: &Path) -> Result<Vec<ToolManifest>> {
    let mut manifests = Vec::new();
    if !dir.exists() {
        return Ok(manifests);
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading tool manifest dir: {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && matches!(
                    p.extension().and_then(|x| x.to_str()),
                    Some("yaml") | Some("yml")
                )
        })
        .collect();
    entries.sort();

    for path in entries {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading manifest: {}", path.display()))?;
        let manifest: ToolManifest = serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing manifest: {}", path.display()))?;
        manifests.push(manifest);
    }
    Ok(manifests)
}

/// Discover external tools from a trusted directory and register them.
///
/// `make_invoker` selects an [`ExternalInvoker`] per manifest (e.g. a subprocess
/// invoker for `command` specs, an HTTP invoker for `url` specs). Manifests whose
/// `status` is set to anything other than `active` are skipped. Duplicate names
/// (already present in the registry, or repeated across manifests) are an error.
///
/// Returns the number of tools registered.
pub fn register_external_tools(
    registry: &mut CanonicalToolRegistry,
    dir: &Path,
    make_invoker: impl Fn(&ToolManifest) -> Result<Box<dyn ExternalInvoker>>,
) -> Result<usize> {
    let manifests = load_manifests_from_dir(dir)?;
    let mut count = 0;
    for manifest in manifests {
        if let Some(status) = &manifest.status
            && !status.eq_ignore_ascii_case("active")
        {
            continue;
        }
        if manifest.command.is_none() && manifest.url.is_none() {
            bail!(
                "tool manifest `{}` has neither `command` nor `url`",
                manifest.name
            );
        }
        if registry.contains(&manifest.name) {
            bail!("duplicate tool name: {}", manifest.name);
        }
        let invoker = make_invoker(&manifest)?;
        registry.register(Box::new(ExternalTool::new(manifest, invoker)));
        count += 1;
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // --- test doubles ------------------------------------------------------

    /// A trivial in-binary canonical tool that echoes its args.
    struct EchoTool;
    impl CanonicalTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes its arguments back."
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "msg": { "type": "string" } } })
        }
        fn run(&self, args: Value) -> Result<Value> {
            Ok(args)
        }
    }

    /// Records the last spec/args it was invoked with, returns a canned value.
    #[derive(Default, Clone)]
    struct FakeInvoker {
        last: Arc<Mutex<Option<(ExternalSpec, Value)>>>,
        reply: Value,
    }
    impl ExternalInvoker for FakeInvoker {
        fn invoke(&self, spec: &ExternalSpec, args: Value) -> Result<Value> {
            *self.last.lock().unwrap() = Some((spec.clone(), args));
            Ok(self.reply.clone())
        }
    }

    /// A fake legacy tool implementing the *old* trait.
    struct FakeLegacy;
    impl crate::tool::Tool for FakeLegacy {
        fn name(&self) -> &str {
            "legacy"
        }
        fn description(&self) -> &str {
            "A legacy tool."
        }
        fn available(&self) -> bool {
            true
        }
        fn run(&self, action: &str, params: &HashMap<String, String>) -> Result<String> {
            Ok(format!("ran {action} with {} params", params.len()))
        }
    }

    // --- ToolType ----------------------------------------------------------

    #[test]
    fn tool_type_round_trips_through_json() {
        for t in [ToolType::Deterministic, ToolType::Predictive] {
            let s = serde_json::to_string(&t).unwrap();
            let back: ToolType = serde_json::from_str(&s).unwrap();
            assert_eq!(t, back);
            assert_eq!(ToolType::parse(t.as_str()).unwrap(), t);
        }
        assert_eq!(
            serde_json::to_string(&ToolType::Predictive).unwrap(),
            "\"predictive\""
        );
        assert!(ToolType::parse("nonsense").is_err());
        assert_eq!(ToolType::default(), ToolType::Deterministic);
    }

    // --- registry ----------------------------------------------------------

    #[test]
    fn registry_register_get_list() {
        let mut reg = CanonicalToolRegistry::new();
        assert!(reg.is_empty());
        reg.register(Box::new(EchoTool));
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("echo"));

        let got = reg.get("echo").expect("echo registered");
        assert_eq!(got.name(), "echo");
        assert!(reg.get("missing").is_none());

        let listed = reg.list();
        assert_eq!(listed.len(), 1);
        let d = &listed[0];
        assert_eq!(d.name, "echo");
        assert_eq!(d.description, "Echoes its arguments back.");
        assert_eq!(d.tool_type, ToolType::Deterministic);
        assert!(d.input_schema.get("properties").is_some());
    }

    #[test]
    fn registered_tool_runs() {
        let mut reg = CanonicalToolRegistry::new();
        reg.register(Box::new(EchoTool));
        let out = reg.get("echo").unwrap().run(json!({"msg": "hi"})).unwrap();
        assert_eq!(out, json!({"msg": "hi"}));
    }

    // --- ExternalTool ------------------------------------------------------

    #[test]
    fn external_tool_delegates_to_invoker() {
        let invoker = FakeInvoker {
            reply: json!({"summary": "ok"}),
            ..Default::default()
        };
        let last = invoker.last.clone();
        let manifest = ToolManifest {
            name: "yt-summarizer".into(),
            description: "Summarize a YouTube video.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
            tool_type: ToolType::Predictive,
            command: None,
            url: Some("http://localhost:9000/summarize".into()),
            dependencies: vec![],
            category: Some("media".into()),
            status: Some("active".into()),
            secrets: vec![],
        };
        let tool = ExternalTool::new(manifest, Box::new(invoker));

        assert_eq!(tool.name(), "yt-summarizer");
        assert_eq!(tool.tool_type(), ToolType::Predictive);

        let out = tool
            .run(json!({"url": "https://youtu.be/x"}))
            .expect("invoke ok");
        assert_eq!(out, json!({"summary": "ok"}));

        let (spec, args) = last.lock().unwrap().clone().unwrap();
        assert_eq!(spec.url.as_deref(), Some("http://localhost:9000/summarize"));
        assert_eq!(args, json!({"url": "https://youtu.be/x"}));
    }

    #[test]
    fn external_tool_validates_required_keys() {
        let invoker = FakeInvoker::default();
        let manifest = ToolManifest {
            name: "needs-url".into(),
            description: String::new(),
            input_schema: json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
            tool_type: ToolType::Deterministic,
            command: None,
            url: Some("http://x".into()),
            dependencies: vec![],
            category: None,
            status: None,
            secrets: vec![],
        };
        let tool = ExternalTool::new(manifest, Box::new(invoker));
        let err = tool.run(json!({"wrong": 1})).unwrap_err();
        assert!(err.to_string().contains("missing required argument: url"));
    }

    #[test]
    fn external_tool_synthesizes_empty_schema() {
        let manifest = ToolManifest {
            name: "noschema".into(),
            description: String::new(),
            input_schema: Value::Null,
            tool_type: ToolType::Deterministic,
            command: Some("true".into()),
            url: None,
            dependencies: vec![],
            category: None,
            status: None,
            secrets: vec![],
        };
        let tool = ExternalTool::new(manifest, Box::new(FakeInvoker::default()));
        assert_eq!(
            tool.input_schema(),
            json!({"type": "object", "properties": {}})
        );
    }

    // --- legacy bridge -----------------------------------------------------

    #[test]
    fn legacy_adapter_maps_run() {
        let adapter = LegacyToolAdapter::new(Box::new(FakeLegacy));
        assert_eq!(adapter.name(), "legacy");
        assert_eq!(adapter.description(), "A legacy tool.");
        assert_eq!(adapter.tool_type(), ToolType::Deterministic);
        assert_eq!(
            adapter.input_schema().get("required").unwrap(),
            &json!(["action"])
        );

        let out = adapter
            .run(json!({"action": "build", "params": {"a": "1", "b": "2"}}))
            .unwrap();
        assert_eq!(out, json!({"output": "ran build with 2 params"}));
    }

    #[test]
    fn legacy_adapter_requires_action() {
        let adapter = LegacyToolAdapter::new(Box::new(FakeLegacy));
        assert!(
            adapter
                .run(json!({"params": {}}))
                .unwrap_err()
                .to_string()
                .contains("action")
        );
    }

    #[test]
    fn legacy_adapter_registers_in_canonical_registry() {
        let mut reg = CanonicalToolRegistry::new();
        reg.register(Box::new(LegacyToolAdapter::new(Box::new(FakeLegacy))));
        let out = reg
            .get("legacy")
            .unwrap()
            .run(json!({"action": "x"}))
            .unwrap();
        assert_eq!(out, json!({"output": "ran x with 0 params"}));
    }

    // --- manifest discovery ------------------------------------------------

    #[test]
    fn discovery_parses_and_registers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("summarizer.yaml"),
            r#"
name: yt-summarizer
description: Summarize a YouTube video.
tool_type: predictive
url: http://localhost:9000/summarize
category: media
status: active
dependencies: [yt-dlp]
input_schema:
  type: object
  properties:
    url:
      type: string
  required: [url]
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("disabled.yaml"),
            "name: nope\ncommand: 'true'\nstatus: deprecated\n",
        )
        .unwrap();
        // A non-yaml file must be ignored.
        std::fs::write(dir.path().join("README.md"), "not a manifest").unwrap();

        let mut reg = CanonicalToolRegistry::new();
        let n = register_external_tools(&mut reg, dir.path(), |_m| {
            Ok(Box::new(FakeInvoker {
                reply: json!({"ok": true}),
                ..Default::default()
            }))
        })
        .unwrap();

        assert_eq!(n, 1, "only the active manifest registers");
        assert!(reg.contains("yt-summarizer"));
        assert!(!reg.contains("nope"));

        let d = &reg.list()[0];
        assert_eq!(d.tool_type, ToolType::Predictive);
        assert_eq!(d.input_schema.get("required").unwrap(), &json!(["url"]));
    }

    #[test]
    fn discovery_rejects_duplicate_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.yaml"), "name: dup\ncommand: 'true'\n").unwrap();
        std::fs::write(dir.path().join("b.yaml"), "name: dup\ncommand: 'true'\n").unwrap();
        let mut reg = CanonicalToolRegistry::new();
        let err = register_external_tools(&mut reg, dir.path(), |_m| {
            Ok(Box::new(FakeInvoker::default()))
        })
        .unwrap_err();
        assert!(err.to_string().contains("duplicate tool name"));
    }

    #[test]
    fn discovery_rejects_manifest_without_command_or_url() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.yaml"), "name: bad\nstatus: active\n").unwrap();
        let mut reg = CanonicalToolRegistry::new();
        let err = register_external_tools(&mut reg, dir.path(), |_m| {
            Ok(Box::new(FakeInvoker::default()))
        })
        .unwrap_err();
        assert!(err.to_string().contains("neither `command` nor `url`"));
    }

    #[test]
    fn discovery_missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let m = load_manifests_from_dir(&missing).unwrap();
        assert!(m.is_empty());
    }

    // --- opt-in flag -------------------------------------------------------

    #[test]
    fn external_tools_disabled_by_default() {
        // The default (var unset) is OFF. We don't mutate process env here to
        // avoid cross-test races; this asserts the unset/false branch only when
        // the var is not present.
        if std::env::var("CO_ENABLE_EXTERNAL_TOOLS").is_err() {
            assert!(!external_tools_enabled());
        }
    }

    // --- real subprocess smoke (offline-safe) ------------------------------

    /// One real subprocess invocation through SubprocessInvoker, using `cat`
    /// (reads stdin, writes it back to stdout). Offline-safe: no network, only a
    /// POSIX coreutil. Skipped automatically if `cat` is unavailable.
    #[test]
    fn subprocess_invoker_roundtrips_via_cat() {
        if Command::new("cat").arg("--version").output().is_err()
            && Command::new("cat").stdin(Stdio::null()).output().is_err()
        {
            eprintln!("skipping: `cat` unavailable");
            return;
        }
        let invoker = SubprocessInvoker;
        let spec = ExternalSpec {
            command: Some("cat".into()),
            url: None,
            secrets: vec![],
        };
        let args = json!({"hello": "world", "n": 3});
        let out = invoker.invoke(&spec, args.clone()).unwrap();
        assert_eq!(out, args, "cat echoes the JSON we wrote to stdin");
    }

    #[test]
    fn subprocess_invoker_maps_nonzero_exit_to_error() {
        let invoker = SubprocessInvoker;
        let spec = ExternalSpec {
            command: Some("false".into()),
            url: None,
            secrets: vec![],
        };
        let err = invoker.invoke(&spec, json!({})).unwrap_err();
        assert!(err.to_string().contains("tool command failed"));
    }
}
