//! Sub-universe discovery and prefix-aware task resolution.
//!
//! A universe is any folder containing `_universe.yaml`. Universes can nest
//! arbitrarily (up to `MAX_DEPTH`). This module walks the tree, builds a
//! `Subspace` registry, and resolves task identifiers against it so that a
//! bare key like `SHN-1` routes to `work/yggdrasil/shandara/SHN-1.md`
//! without the caller spelling out the nesting.

use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

/// A discovered sub-universe within a space.
#[derive(Debug, Clone)]
pub struct Subspace {
    /// Short name of the subspace (directory name, or space name for root).
    pub key: String,
    /// Path relative to the `work/` (or `data/`) parent directory.
    /// e.g. `"yggdrasil"` for root, `"yggdrasil/shandara"` for a child.
    pub rel_path: PathBuf,
    /// Absolute path to the subspace directory.
    pub abs_path: PathBuf,
    /// Task prefix declared in `_universe.yaml` (e.g. `"SHN"`), or inferred.
    pub prefix: String,
    /// Key of the parent subspace, from `_universe.yaml.parent` or inferred.
    pub parent: Option<String>,
    /// Version from `_universe.yaml`, if present.
    pub version: Option<String>,
}

/// The result of resolving a task identifier to a concrete subspace + path.
#[derive(Debug, Clone)]
pub struct ResolvedTask {
    /// Canonical task key (e.g. `"SHN-1"`).
    pub key: String,
    /// The subspace that owns this task.
    pub subspace: Subspace,
    /// Expected path to the task spec file.
    pub spec_path: PathBuf,
}

const SKIP_DIRS: &[&str] = &[
    ".worktrees",
    ".git",
    "node_modules",
    "target",
    "CHANGELOG-PENDING",
];
const MAX_DEPTH: usize = 8;

/// Discover all subspaces under `workdir/work/<space>/` (or `data/<space>/`).
///
/// Always returns at least the root subspace when the space directory exists.
/// Returns an empty vec only if the space directory cannot be found.
pub fn discover_subspaces(workdir: &Path, space: &str) -> Vec<Subspace> {
    for parent_name in &["work", "data"] {
        let work_parent = workdir.join(parent_name);
        let root_dir = work_parent.join(space);
        if !root_dir.is_dir() {
            continue;
        }
        return collect_subspaces(&work_parent, &root_dir, space);
    }
    vec![]
}

fn collect_subspaces(work_parent: &Path, root_dir: &Path, space: &str) -> Vec<Subspace> {
    let mut result = Vec::new();

    // Always register root subspace, synthesising prefix when needed.
    let yaml_path = root_dir.join("_universe.yaml");
    let (prefix_opt, version, _parent) = read_universe_yaml_fields(if yaml_path.exists() {
        Some(&yaml_path)
    } else {
        None
    });

    let prefix = prefix_opt
        .or_else(|| lookup_prefix_table(space).map(str::to_string))
        .or_else(|| infer_prefix_from_files(root_dir))
        .unwrap_or_else(|| space.to_uppercase());

    result.push(Subspace {
        key: space.to_string(),
        rel_path: PathBuf::from(space),
        abs_path: root_dir.to_path_buf(),
        prefix,
        parent: None,
        version,
    });

    walk_for_subspaces(work_parent, root_dir, space, 1, &mut result);
    result
}

fn walk_for_subspaces(
    work_parent: &Path,
    dir: &Path,
    parent_key: &str,
    depth: usize,
    result: &mut Vec<Subspace>,
) {
    if depth > MAX_DEPTH {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if SKIP_DIRS.contains(&name) {
            continue;
        }

        let yaml_path = path.join("_universe.yaml");
        if yaml_path.exists() {
            let key = name.to_string();
            let rel_path = path
                .strip_prefix(work_parent)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| PathBuf::from(&key));

            let (prefix_opt, version, parent_from_yaml) =
                read_universe_yaml_fields(Some(&yaml_path));
            let prefix = prefix_opt.unwrap_or_else(|| key.to_uppercase());
            let parent = parent_from_yaml.or_else(|| Some(parent_key.to_string()));

            let child_key = key.clone();
            result.push(Subspace {
                key,
                rel_path,
                abs_path: path.clone(),
                prefix,
                parent,
                version,
            });

            walk_for_subspaces(work_parent, &path, &child_key, depth + 1, result);
        } else {
            // Descend even without _universe.yaml — a subspace may be deeper.
            walk_for_subspaces(work_parent, &path, parent_key, depth + 1, result);
        }
    }
}

/// Resolve a bare number or full key to a [`ResolvedTask`].
///
/// Resolution rules:
/// - Input contains `-` (e.g. `"SHN-1"`): prefix uniquely identifies the
///   subspace. Falls back to root when no subspace claims it; errors on
///   ambiguity (two subspaces share the prefix).
/// - Bare number (e.g. `"1"`): always resolves to the root space prefix.
///   Callers that implement `-u <key>` should expand the bare number to
///   `{subspace.prefix}-{number}` before calling this function.
///
/// When `subspaces` is empty, falls back to the legacy string-based resolver
/// for full backward compatibility.
pub fn resolve_task_id(
    input: &str,
    space: &str,
    workdir: &Path,
    subspaces: &[Subspace],
) -> Result<ResolvedTask> {
    if subspaces.is_empty() {
        return resolve_legacy(input, space, workdir);
    }

    if input.contains('-') {
        let dash = input.find('-').expect("contains checked above");
        let prefix = input[..dash].to_uppercase();
        let key = input.to_uppercase();

        let matches: Vec<&Subspace> = subspaces
            .iter()
            .filter(|s| s.prefix.to_uppercase() == prefix)
            .collect();

        return match matches.len() {
            0 => {
                let root = root_subspace(subspaces, space);
                let spec_path = root.abs_path.join(format!("{}.md", key));
                Ok(ResolvedTask {
                    key,
                    subspace: root.clone(),
                    spec_path,
                })
            }
            1 => {
                let sub = matches[0];
                let spec_path = sub.abs_path.join(format!("{}.md", key));
                Ok(ResolvedTask {
                    key,
                    subspace: sub.clone(),
                    spec_path,
                })
            }
            _ => {
                let names: Vec<String> = matches.iter().map(|s| s.key.clone()).collect();
                Err(anyhow!(
                    "ambiguous prefix '{}' claimed by {}; specify -u <key>",
                    prefix,
                    names.join(", ")
                ))
            }
        };
    }

    // Bare number — always resolves to root space.
    let root = root_subspace(subspaces, space);
    let key = format!("{}-{}", root.prefix, input);
    let spec_path = root.abs_path.join(format!("{}.md", key));
    Ok(ResolvedTask {
        key,
        subspace: root.clone(),
        spec_path,
    })
}

fn root_subspace<'a>(subspaces: &'a [Subspace], space: &str) -> &'a Subspace {
    subspaces
        .iter()
        .find(|s| s.key == space)
        .or_else(|| subspaces.first())
        .expect("non-empty slice (checked before call)")
}

/// Legacy fallback when no subspaces were discovered.
///
/// Preserves the CO-276 resolution order: `_universe.yaml` → hardcoded table
/// → file-scan → uppercase space name.
fn resolve_legacy(input: &str, space: &str, workdir: &Path) -> Result<ResolvedTask> {
    let abs_path = ["work", "data"]
        .iter()
        .map(|p| workdir.join(p).join(space))
        .find(|p| p.is_dir())
        .unwrap_or_else(|| workdir.join("work").join(space));

    let key = if input.contains('-') {
        input.to_uppercase()
    } else {
        let yaml_path = abs_path.join("_universe.yaml");
        let prefix = read_universe_yaml_fields(if yaml_path.exists() {
            Some(&yaml_path)
        } else {
            None
        })
        .0
        .or_else(|| lookup_prefix_table(space).map(str::to_string))
        .or_else(|| infer_prefix_from_files(&abs_path))
        .unwrap_or_else(|| space.to_uppercase());
        format!("{}-{}", prefix, input)
    };

    let inferred_prefix = key
        .find('-')
        .map(|i| key[..i].to_string())
        .unwrap_or_else(|| space.to_uppercase());

    let root = Subspace {
        key: space.to_string(),
        rel_path: PathBuf::from(space),
        abs_path: abs_path.clone(),
        prefix: inferred_prefix,
        parent: None,
        version: None,
    };
    let spec_path = abs_path.join(format!("{}.md", key));
    Ok(ResolvedTask {
        key,
        subspace: root,
        spec_path,
    })
}

/// Read `task_prefix`, `version`, and `parent` from a `_universe.yaml` file.
pub(crate) fn read_universe_yaml_fields(
    path: Option<&Path>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(path) = path else {
        return (None, None, None);
    };
    let Ok(content) = fs::read_to_string(path) else {
        return (None, None, None);
    };
    let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
        return (None, None, None);
    };
    let map = yaml.as_mapping();

    let get = |key: &str| -> Option<String> {
        map.and_then(|m| m.get(serde_yaml::Value::String(key.into())))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };

    (get("task_prefix"), get("version"), get("parent"))
}

pub(crate) fn lookup_prefix_table(space: &str) -> Option<&'static str> {
    match space {
        "co" => Some("CO"),
        "yggdrasil" => Some("YG"),
        "rfq" => Some("RFQ"),
        "qb" => Some("QB"),
        "artelonga" => Some("AL"),
        _ => None,
    }
}

pub(crate) fn infer_prefix_from_files(dir: &Path) -> Option<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.ends_with(".md") {
            continue;
        }
        let stem = &s[..s.len() - 3];
        if let Some(dash) = stem.find('-') {
            let prefix = &stem[..dash];
            let num = &stem[dash + 1..];
            if !prefix.is_empty()
                && prefix.chars().all(|c| c.is_ascii_uppercase())
                && !num.is_empty()
                && num.chars().all(|c| c.is_ascii_digit())
            {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(label: &str) -> PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        std::env::temp_dir().join(format!("co-universe-{}-{}", label, ts))
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn root_only_no_nesting() {
        let tmp = tmp_dir("root-only");
        let space_dir = tmp.join("work").join("yg");
        fs::create_dir_all(&space_dir).unwrap();
        write(
            &space_dir.join("_universe.yaml"),
            "task_prefix: YG\nversion: 1.0.0\n",
        );

        let subs = discover_subspaces(&tmp, "yg");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].key, "yg");
        assert_eq!(subs[0].prefix, "YG");
        assert!(subs[0].parent.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn two_level_depth() {
        let tmp = tmp_dir("two-level");
        let root = tmp.join("work").join("yg");
        let child = root.join("sh");
        fs::create_dir_all(&child).unwrap();
        write(&root.join("_universe.yaml"), "task_prefix: YG\n");
        write(
            &child.join("_universe.yaml"),
            "task_prefix: SH\nparent: yg\n",
        );

        let subs = discover_subspaces(&tmp, "yg");
        assert_eq!(subs.len(), 2);
        assert!(subs.iter().any(|s| s.key == "yg" && s.prefix == "YG"));
        assert!(subs.iter().any(|s| s.key == "sh" && s.prefix == "SH"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn three_level_depth() {
        let tmp = tmp_dir("three-level");
        let root = tmp.join("work").join("yg");
        let child = root.join("sh");
        let grandchild = child.join("exp");
        fs::create_dir_all(&grandchild).unwrap();
        write(&root.join("_universe.yaml"), "task_prefix: YG\n");
        write(&child.join("_universe.yaml"), "task_prefix: SH\n");
        write(&grandchild.join("_universe.yaml"), "task_prefix: EXP\n");

        let subs = discover_subspaces(&tmp, "yg");
        assert_eq!(subs.len(), 3);
        assert!(subs.iter().any(|s| s.key == "exp" && s.prefix == "EXP"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn skips_excluded_directories() {
        let tmp = tmp_dir("skip-dirs");
        let root = tmp.join("work").join("yg");
        let worktrees = root.join(".worktrees");
        let git = root.join(".git");
        let valid = root.join("valid-child");
        fs::create_dir_all(&worktrees).unwrap();
        fs::create_dir_all(&git).unwrap();
        fs::create_dir_all(&valid).unwrap();
        write(&root.join("_universe.yaml"), "task_prefix: YG\n");
        write(&worktrees.join("_universe.yaml"), "task_prefix: WT\n");
        write(&git.join("_universe.yaml"), "task_prefix: GIT\n");
        write(&valid.join("_universe.yaml"), "task_prefix: VC\n");

        let subs = discover_subspaces(&tmp, "yg");
        assert!(
            !subs.iter().any(|s| s.prefix == "WT"),
            ".worktrees should be skipped"
        );
        assert!(
            !subs.iter().any(|s| s.prefix == "GIT"),
            ".git should be skipped"
        );
        assert!(
            subs.iter().any(|s| s.prefix == "VC"),
            "valid child should be included"
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}
