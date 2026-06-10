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
fn rust_patterns() -> Vec<Pattern> {
    vec![
        Pattern {
            needle: "format!(\"SELECT",
            severity: Severity::High,
            category: Category::SqlInjection,
            description: "Possible SQL injection: string interpolation inside SELECT query",
            cwe: Some("CWE-89"),
        },
        Pattern {
            needle: "format!(\"INSERT",
            severity: Severity::High,
            category: Category::SqlInjection,
            description: "Possible SQL injection: string interpolation inside INSERT query",
            cwe: Some("CWE-89"),
        },
        Pattern {
            needle: "format!(\"UPDATE",
            severity: Severity::High,
            category: Category::SqlInjection,
            description: "Possible SQL injection: string interpolation inside UPDATE query",
            cwe: Some("CWE-89"),
        },
        Pattern {
            needle: "format!(\"DELETE",
            severity: Severity::High,
            category: Category::SqlInjection,
            description: "Possible SQL injection: string interpolation inside DELETE query",
            cwe: Some("CWE-89"),
        },
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

    /// Run `git diff --name-only base_ref..head_ref` and return changed source files.
    async fn changed_files(base_ref: &str, head_ref: &str) -> Vec<PathBuf> {
        let output = tokio::process::Command::new("git")
            .args(["diff", "--name-only", &format!("{base_ref}...{head_ref}")])
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
        let b = backend();
        let findings = b.scan_content(
            "routes.rs",
            r#"conn.query("SELECT * FROM users WHERE id = ?1", params![id])"#,
            true,
        );
        assert!(findings.is_empty());
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
