//! CO-388: `LocalGrepBackend` — pattern-based static analysis.
//!
//! Default security backend. Always available, no external API required.
//! Scans Rust, TypeScript, and SQL source files for common vulnerability
//! patterns. Prioritises TypeScript paths (XSS surface) and auth routes.
//!
//! Cache: results are keyed by SHA-256 of the file content so unchanged
//! files are not re-scanned on successive runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tracing::warn;

use super::{AuditResult, Category, Finding, PatchSuggestion, SecurityAuditBackend, Severity};

// ---------------------------------------------------------------------------
// Pattern database
// ---------------------------------------------------------------------------

struct Pattern {
    needle: &'static str,
    severity: Severity,
    category: Category,
    description: &'static str,
    cwe: Option<&'static str>,
}

/// Patterns for Rust source files.
///
/// CWE-89 (SQL injection) is NOT a simple-substring pattern — it lives in
/// [`sql_format_finding`] because a bare `format!("DELETE …")` is just as
/// likely an HTTP-verb error string or a log line as a SQL query. See that
/// function for the SQL-context heuristic (CO-447).
fn rust_patterns() -> Vec<Pattern> {
    vec![
        Pattern {
            needle: "unwrap_or_else(|_| panic!",
            severity: Severity::Low,
            category: Category::Other("panic-in-production".into()),
            description: "Panic in production code may crash the server",
            cwe: None,
        },
        Pattern {
            needle: "unsafe {",
            severity: Severity::Low,
            category: Category::Other("unsafe-block".into()),
            description: "Unsafe block detected — review for memory safety",
            cwe: None,
        },
        Pattern {
            needle: "std::env::var(\"SECRET",
            severity: Severity::Medium,
            category: Category::HardcodedSecret,
            description: "Secret loaded from environment — ensure not hardcoded in calling code",
            cwe: Some("CWE-798"),
        },
    ]
}

/// Patterns for TypeScript/JavaScript source files.
fn ts_patterns() -> Vec<Pattern> {
    vec![
        Pattern {
            needle: ".innerHTML =",
            severity: Severity::High,
            category: Category::Xss,
            description: "XSS risk: direct innerHTML assignment without sanitisation",
            cwe: Some("CWE-79"),
        },
        Pattern {
            needle: ".innerHTML+=",
            severity: Severity::High,
            category: Category::Xss,
            description: "XSS risk: innerHTML append without sanitisation",
            cwe: Some("CWE-79"),
        },
        Pattern {
            needle: "document.write(",
            severity: Severity::High,
            category: Category::Xss,
            description: "XSS risk: document.write injects unsanitised HTML",
            cwe: Some("CWE-79"),
        },
        Pattern {
            needle: "eval(",
            severity: Severity::Critical,
            category: Category::CommandInjection,
            description: "eval() executes arbitrary code — high injection risk",
            cwe: Some("CWE-95"),
        },
        Pattern {
            needle: "dangerouslySetInnerHTML",
            severity: Severity::Medium,
            category: Category::Xss,
            description: "React dangerouslySetInnerHTML — ensure content is sanitised",
            cwe: Some("CWE-79"),
        },
        Pattern {
            needle: "localStorage.setItem(\"token",
            severity: Severity::Medium,
            category: Category::HardcodedSecret,
            description: "JWT stored in localStorage — XSS can exfiltrate tokens",
            cwe: Some("CWE-922"),
        },
        Pattern {
            needle: "fetch(`",
            severity: Severity::Low,
            category: Category::Ssrf,
            description: "Dynamic fetch URL — verify template literal cannot include user input",
            cwe: Some("CWE-918"),
        },
    ]
}

/// Patterns applied to all file types.
fn universal_patterns() -> Vec<Pattern> {
    vec![
        Pattern {
            needle: "TODO: security",
            severity: Severity::Info,
            category: Category::Other("todo".into()),
            description: "Security TODO comment found — track to resolution",
            cwe: None,
        },
        Pattern {
            needle: "FIXME: auth",
            severity: Severity::Medium,
            category: Category::AuthBypass,
            description: "Auth FIXME found — must be resolved before release",
            cwe: Some("CWE-306"),
        },
        Pattern {
            needle: "// nosec",
            severity: Severity::Info,
            category: Category::Other("nosec-annotation".into()),
            description: "Security suppression annotation — verify intentional",
            cwe: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// CWE-89: context-aware SQL-injection heuristic (CO-447)
// ---------------------------------------------------------------------------

/// SQL statement verbs that open a query.
const SQL_VERBS: &[&str] = &["SELECT", "INSERT", "UPDATE", "DELETE"];

/// Clause keywords that only appear inside a real SQL statement. A `format!`
/// string that opens with a SQL verb is only flagged when one of these follows
/// the verb in the SAME string literal. This is what separates a genuine query
/// (`format!("SELECT {c} FROM t WHERE x={x}")`) from an inert HTTP-verb error
/// string (`format!("DELETE vault/{path}")`) or a log line
/// (`format!("UPDATE failed for {id}")`), which were CWE-89 false positives in
/// the CO-445 wave.
const SQL_CONTEXT_TOKENS: &[&str] = &["FROM", "INTO", "SET", "WHERE", "VALUES"];

/// Mirrors the `pr-route.yml` shell regex
/// `format!\("(SELECT|INSERT|UPDATE|DELETE)[^"]*(FROM|INTO|SET|WHERE|VALUES)`.
///
/// Returns the matched SQL verb when `line` contains a `format!` whose string
/// literal both *begins* with a SQL verb and *also* contains a SQL clause
/// keyword. Matching is case-sensitive (CO writes SQL keywords uppercase) and
/// scoped to the format string's contents, so a clause keyword in unrelated
/// trailing code cannot trigger it.
fn sql_format_verb(line: &str) -> Option<&'static str> {
    const OPEN: &str = "format!(\"";
    let mut rest = line;
    while let Some(idx) = rest.find(OPEN) {
        let after = &rest[idx + OPEN.len()..];
        // The format string's contents end at the next quote.
        let content = match after.find('"') {
            Some(end) => &after[..end],
            None => after,
        };
        for verb in SQL_VERBS {
            if let Some(tail) = content.strip_prefix(verb)
                && SQL_CONTEXT_TOKENS.iter().any(|t| tail.contains(t))
            {
                return Some(verb);
            }
        }
        rest = after;
    }
    None
}

/// Build a CWE-89 finding for `line` if it looks like an interpolated SQL query.
fn sql_format_finding(file_path: &str, line: &str, line_no: usize) -> Option<Finding> {
    let verb = sql_format_verb(line)?;
    let description: &str = match verb {
        "SELECT" => "Possible SQL injection: string interpolation inside SELECT query",
        "INSERT" => "Possible SQL injection: string interpolation inside INSERT query",
        "UPDATE" => "Possible SQL injection: string interpolation inside UPDATE query",
        _ => "Possible SQL injection: string interpolation inside DELETE query",
    };
    let mut f = Finding::new(
        Severity::High,
        Category::SqlInjection,
        file_path,
        (line_no, line_no),
        description,
    );
    f.cwe = Some("CWE-89".to_string());
    Some(f)
}

// ---------------------------------------------------------------------------
// File hash cache
// ---------------------------------------------------------------------------

fn file_hash(content: &[u8]) -> String {
    let h = Sha256::digest(content);
    format!("{:x}", h)[..16].to_string()
}

// ---------------------------------------------------------------------------
// LocalGrepBackend
// ---------------------------------------------------------------------------

pub struct LocalGrepBackend {
    /// Cache: file_hash → findings. Avoids re-scanning unchanged files.
    cache: Mutex<HashMap<String, Vec<Finding>>>,
}

impl LocalGrepBackend {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Scan a single file's `content` for vulnerability patterns.
    /// Returns any findings, using `file_path` as the label.
    pub fn scan_content(&self, file_path: &str, content: &str, bypass_cache: bool) -> Vec<Finding> {
        let content_bytes = content.as_bytes();
        let hash = file_hash(content_bytes);

        if !bypass_cache {
            let cached = self.cache.lock().ok().and_then(|c| c.get(&hash).cloned());
            if let Some(cached) = cached {
                return cached;
            }
        }

        let ext = Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let patterns: Vec<Pattern> = match ext {
            "rs" => {
                let mut p = rust_patterns();
                p.extend(universal_patterns());
                p
            }
            "ts" | "js" | "tsx" | "jsx" => {
                let mut p = ts_patterns();
                p.extend(universal_patterns());
                p
            }
            _ => universal_patterns(),
        };

        let mut findings = Vec::new();

        for (line_idx, line) in content.lines().enumerate() {
            let line_no = line_idx + 1;
            for pat in &patterns {
                if line.contains(pat.needle) {
                    let mut f = Finding::new(
                        pat.severity.clone(),
                        pat.category.clone(),
                        file_path,
                        (line_no, line_no),
                        pat.description,
                    );
                    if let Some(cwe) = pat.cwe {
                        f.cwe = Some(cwe.to_string());
                    }
                    findings.push(f);
                }
            }
            // CWE-89 requires SQL context, so it is not a plain-substring
            // pattern (CO-447). Only meaningful for Rust source.
            if ext == "rs"
                && let Some(f) = sql_format_finding(file_path, line, line_no)
            {
                findings.push(f);
            }
        }

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(hash, findings.clone());
        }

        findings
    }

    /// Scan a list of file paths.
    async fn scan_files(&self, paths: Vec<PathBuf>) -> Vec<Finding> {
        let mut findings = Vec::new();
        for path in paths {
            let path_str = path.to_string_lossy().to_string();
            match tokio::fs::read(&path).await {
                Ok(bytes) => {
                    if let Ok(content) = std::str::from_utf8(&bytes) {
                        findings.extend(self.scan_content(&path_str, content, false));
                    }
                }
                Err(e) => {
                    warn!("security: cannot read {path_str}: {e}");
                }
            }
        }
        findings
    }

    /// Walk `root` recursively and collect scannable source files.
    async fn collect_files(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&dir).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    // Skip common non-source directories.
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if matches!(name, "target" | "node_modules" | ".git" | "dist" | "build") {
                        continue;
                    }
                    stack.push(path);
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|ext| matches!(ext, "rs" | "ts" | "js" | "tsx" | "jsx" | "sql"))
                {
                    files.push(path);
                }
            }
        }
        files
    }

    /// Run `git diff --name-only base_ref...head_ref` and return changed source files.
    async fn changed_files(base_ref: &str, head_ref: &str) -> Vec<PathBuf> {
        // CO-388 (security hardening): refs are attacker-controllable (they
        // arrive from the scan request body). Reject anything outside the safe
        // git-ref charset to prevent argument injection (e.g. a ref of
        // `--output=...`). The trailing `--` terminates option/revision parsing
        // so the range can never be reinterpreted as a pathspec or option.
        if !super::is_safe_git_ref(base_ref) || !super::is_safe_git_ref(head_ref) {
            warn!("security: refusing git diff with unsafe ref(s): {base_ref:?}...{head_ref:?}");
            return vec![];
        }
        let output = tokio::process::Command::new("git")
            .args([
                "diff",
                "--name-only",
                &format!("{base_ref}...{head_ref}"),
                "--",
            ])
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| {
                    let ext = Path::new(l)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    matches!(ext, "rs" | "ts" | "js" | "tsx" | "jsx" | "sql")
                })
                .map(PathBuf::from)
                .collect(),
            Ok(out) => {
                warn!(
                    "security: git diff failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                vec![]
            }
            Err(e) => {
                warn!("security: git not available: {e}");
                vec![]
            }
        }
    }
}

impl Default for LocalGrepBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecurityAuditBackend for LocalGrepBackend {
    async fn scan_diff(&self, base_ref: &str, head_ref: &str) -> AuditResult {
        let paths = Self::changed_files(base_ref, head_ref).await;
        Ok(self.scan_files(paths).await)
    }

    async fn scan_full(&self, repo_path: &Path) -> AuditResult {
        let paths = Self::collect_files(repo_path).await;
        Ok(self.scan_files(paths).await)
    }

    async fn suggest_patch(
        &self,
        finding: &Finding,
    ) -> Result<Option<PatchSuggestion>, anyhow::Error> {
        let suggestion = match &finding.category {
            Category::SqlInjection => Some(PatchSuggestion {
                description:
                    "Use parameterised queries (rusqlite params![]) instead of string interpolation"
                        .into(),
                diff: None,
            }),
            Category::Xss => Some(PatchSuggestion {
                description:
                    "Sanitise HTML with DOMPurify before innerHTML assignment, or use textContent"
                        .into(),
                diff: None,
            }),
            Category::HardcodedSecret => Some(PatchSuggestion {
                description: "Move secret to environment variable; never commit credentials".into(),
                diff: None,
            }),
            _ => None,
        };
        Ok(suggestion)
    }

    fn name(&self) -> &'static str {
        "local-grep"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> LocalGrepBackend {
        LocalGrepBackend::new()
    }

    #[test]
    fn detects_innerHTML_in_ts() {
        let b = backend();
        let findings = b.scan_content("app.ts", "element.innerHTML = userInput;", true);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].category.as_str(), "xss");
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn detects_sql_injection_in_rust() {
        let b = backend();
        let findings = b.scan_content(
            "routes.rs",
            r#"let q = format!("SELECT * FROM users WHERE id = {}", id);"#,
            true,
        );
        assert!(!findings.is_empty());
        assert_eq!(findings[0].category.as_str(), "sql_injection");
    }

    #[test]
    fn no_false_positive_on_safe_sql() {
        // Const column-list + bound params (?1) is the canonical safe pattern —
        // it is not a `format!`, so it must never produce a CWE-89 finding.
        let b = backend();
        let findings = b.scan_content(
            "routes.rs",
            r#"conn.query("SELECT * FROM users WHERE id = ?1", params![id])"#,
            true,
        );
        assert!(findings.is_empty());
    }

    fn sql_findings(b: &LocalGrepBackend, content: &str) -> usize {
        b.scan_content("routes.rs", content, true)
            .into_iter()
            .filter(|f| f.category.as_str() == "sql_injection")
            .count()
    }

    #[test]
    fn no_false_positive_on_http_verb_string() {
        // CO-51 / CO-445 false positive: a DELETE *HTTP verb* error string is
        // not SQL — no FROM/INTO/SET/WHERE/VALUES follows the verb.
        let b = backend();
        assert_eq!(
            sql_findings(&b, r#"return Err(format!("DELETE vault/{path}"));"#),
            0
        );
    }

    #[test]
    fn no_false_positive_on_log_string() {
        // A log line that merely starts with a SQL-looking verb has no SQL
        // clause keyword and must not be flagged.
        let b = backend();
        assert_eq!(
            sql_findings(
                &b,
                r#"tracing::warn!("{}", format!("UPDATE failed for {id}"));"#
            ),
            0
        );
    }

    #[test]
    fn detects_interpolated_sql_with_context() {
        // Real interpolated SQL: verb + clause keyword in the same format string.
        let b = backend();
        assert_eq!(
            sql_findings(&b, r#"let q = format!("SELECT {c} FROM t WHERE x = {x}");"#),
            1
        );
    }

    #[test]
    fn detects_interpolated_insert_and_update() {
        let b = backend();
        assert_eq!(
            sql_findings(&b, r#"let q = format!("INSERT INTO t (a) VALUES ({a})");"#),
            1
        );
        assert_eq!(
            sql_findings(
                &b,
                r#"let q = format!("UPDATE t SET a = {a} WHERE id = {id}");"#
            ),
            1
        );
    }

    #[test]
    fn cache_returns_same_findings() {
        let b = backend();
        let content = "element.innerHTML = x;";
        let first = b.scan_content("a.ts", content, false);
        let second = b.scan_content("a.ts", content, false);
        assert_eq!(first.len(), second.len());
    }

    #[test]
    fn detects_eval_in_ts() {
        let b = backend();
        let findings = b.scan_content("util.ts", "eval(userCode);", true);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, Severity::Critical);
    }
}
