//! Tests for prefix-aware task resolution across a nested subspace tree.

use co_auto::{discover_subspaces, resolve_task_id};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp_dir(label: &str) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("co-recursive-{}-{}", label, ts))
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// Build the canonical 2-level yggdrasil fixture:
///
/// ```text
/// work/yggdrasil/
/// ├── _universe.yaml        task_prefix: YG
/// ├── YG-64.md
/// └── shandara/
///     ├── _universe.yaml    task_prefix: SHN
///     └── SHN-1.md
/// ```
fn build_yg_shandara(workdir: &Path) {
    let root = workdir.join("work").join("yggdrasil");
    let shandara = root.join("shandara");
    fs::create_dir_all(&shandara).unwrap();

    write(
        &root.join("_universe.yaml"),
        "task_prefix: YG\nversion: 1.1.0\n",
    );
    write(
        &root.join("YG-64.md"),
        "---\nid: 64\ntitle: Root task\nstatus: todo\npriority: medium\n---\n",
    );
    write(
        &shandara.join("_universe.yaml"),
        "task_prefix: SHN\nversion: 0.3.1\nparent: yggdrasil\n",
    );
    write(
        &shandara.join("SHN-1.md"),
        "---\nid: 1\ntitle: Shandara task\nstatus: todo\npriority: medium\n---\n",
    );
}

// ---------------------------------------------------------------------------
// 2-level: SHN-1 resolves to shandara
// ---------------------------------------------------------------------------

#[test]
fn two_level_prefixed_key_routes_to_subspace() {
    let tmp = tmp_dir("2level-prefixed");
    build_yg_shandara(&tmp);

    let subs = discover_subspaces(&tmp, "yggdrasil");
    let rt = resolve_task_id("SHN-1", "yggdrasil", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "SHN-1");
    assert_eq!(rt.subspace.key, "shandara");
    assert_eq!(
        rt.spec_path,
        tmp.join("work")
            .join("yggdrasil")
            .join("shandara")
            .join("SHN-1.md")
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn two_level_lowercase_prefixed_key_uppercased() {
    let tmp = tmp_dir("2level-lower");
    build_yg_shandara(&tmp);

    let subs = discover_subspaces(&tmp, "yggdrasil");
    let rt = resolve_task_id("shn-1", "yggdrasil", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "SHN-1");
    assert_eq!(rt.subspace.key, "shandara");

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// 2-level: YG-64 (root prefix) resolves to root, not shandara
// ---------------------------------------------------------------------------

#[test]
fn two_level_root_prefix_stays_in_root() {
    let tmp = tmp_dir("2level-root");
    build_yg_shandara(&tmp);

    let subs = discover_subspaces(&tmp, "yggdrasil");
    let rt = resolve_task_id("YG-64", "yggdrasil", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "YG-64");
    assert_eq!(rt.subspace.key, "yggdrasil");

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Bare number (no -u) resolves to root
// ---------------------------------------------------------------------------

#[test]
fn bare_number_resolves_to_root() {
    let tmp = tmp_dir("bare-root");
    build_yg_shandara(&tmp);

    let subs = discover_subspaces(&tmp, "yggdrasil");
    let rt = resolve_task_id("64", "yggdrasil", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "YG-64");
    assert_eq!(rt.subspace.key, "yggdrasil");

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// -u shandara 1  →  SHN-1 in shandara
// (bare-number expansion happens before calling resolve_task_id, as run() does)
// ---------------------------------------------------------------------------

#[test]
fn bare_number_with_subspace_key_expands_to_subspace_prefix() {
    let tmp = tmp_dir("bare-u");
    build_yg_shandara(&tmp);

    let subs = discover_subspaces(&tmp, "yggdrasil");
    let shandara = subs.iter().find(|s| s.key == "shandara").unwrap();
    let expanded = format!("{}-{}", shandara.prefix, "1");

    let rt = resolve_task_id(&expanded, "yggdrasil", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "SHN-1");
    assert_eq!(rt.subspace.key, "shandara");

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// 3-level depth: grandchild resolves
// ---------------------------------------------------------------------------

#[test]
fn three_level_grandchild_resolves() {
    let tmp = tmp_dir("3level");
    let root = tmp.join("work").join("yg");
    let child = root.join("sh");
    let grandchild = child.join("exp");
    fs::create_dir_all(&grandchild).unwrap();

    write(&root.join("_universe.yaml"), "task_prefix: YG\n");
    write(&child.join("_universe.yaml"), "task_prefix: SH\n");
    write(&grandchild.join("_universe.yaml"), "task_prefix: EXP\n");
    write(
        &grandchild.join("EXP-1.md"),
        "---\nid: 1\ntitle: Expansion task\nstatus: todo\npriority: medium\n---\n",
    );

    let subs = discover_subspaces(&tmp, "yg");
    let rt = resolve_task_id("EXP-1", "yg", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "EXP-1");
    assert_eq!(rt.subspace.key, "exp");
    assert_eq!(rt.spec_path, grandchild.join("EXP-1.md"));

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Ambiguous prefix: two subspaces share the same prefix
// ---------------------------------------------------------------------------

#[test]
fn ambiguous_prefix_returns_error_with_hint() {
    let tmp = tmp_dir("ambiguous");
    let root = tmp.join("work").join("yg");
    let alpha = root.join("alpha");
    let beta = root.join("beta");
    fs::create_dir_all(&alpha).unwrap();
    fs::create_dir_all(&beta).unwrap();

    write(&root.join("_universe.yaml"), "task_prefix: YG\n");
    write(&alpha.join("_universe.yaml"), "task_prefix: XX\n");
    write(&beta.join("_universe.yaml"), "task_prefix: XX\n");

    let subs = discover_subspaces(&tmp, "yg");
    let err = resolve_task_id("XX-1", "yg", &tmp, &subs).unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("ambiguous"),
        "expected 'ambiguous' in error, got: {msg}"
    );
    assert!(
        msg.contains("specify -u"),
        "expected 'specify -u' hint in error, got: {msg}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Unknown prefix falls back to root
// ---------------------------------------------------------------------------

#[test]
fn unknown_prefix_falls_back_to_root() {
    let tmp = tmp_dir("unknown-prefix");
    build_yg_shandara(&tmp);

    let subs = discover_subspaces(&tmp, "yggdrasil");
    let rt = resolve_task_id("UNKNOWN-99", "yggdrasil", &tmp, &subs).unwrap();

    assert_eq!(rt.key, "UNKNOWN-99");
    assert_eq!(rt.subspace.key, "yggdrasil");

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Backward compatibility: empty subspaces slice still works (legacy path)
// ---------------------------------------------------------------------------

#[test]
fn empty_subspaces_legacy_known_space() {
    let wd = Path::new("/tmp");
    let rt = resolve_task_id("272", "co", wd, &[]).unwrap();
    assert_eq!(rt.key, "CO-272");
}

#[test]
fn empty_subspaces_legacy_full_key() {
    let wd = Path::new("/tmp");
    let rt = resolve_task_id("CO-272", "co", wd, &[]).unwrap();
    assert_eq!(rt.key, "CO-272");
}
