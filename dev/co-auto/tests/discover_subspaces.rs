//! Tests for the `discover_subspaces` tree walker.

use co_auto::discover_subspaces;
use std::fs;
use std::path::{Path, PathBuf};

fn tmp_dir(label: &str) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("co-discover-{}-{}", label, ts))
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn empty_when_space_does_not_exist() {
    let tmp = tmp_dir("no-space");
    let subs = discover_subspaces(&tmp, "nonexistent");
    assert!(subs.is_empty());
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn root_only_returns_one_subspace() {
    let tmp = tmp_dir("root-only");
    let root = tmp.join("work").join("co");
    fs::create_dir_all(&root).unwrap();
    write(
        &root.join("_universe.yaml"),
        "task_prefix: CO\nversion: 2.30.0\n",
    );

    let subs = discover_subspaces(&tmp, "co");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].key, "co");
    assert_eq!(subs[0].prefix, "CO");
    assert_eq!(subs[0].version.as_deref(), Some("2.30.0"));
    assert!(subs[0].parent.is_none());
    assert_eq!(subs[0].abs_path, root);
    assert_eq!(subs[0].rel_path, PathBuf::from("co"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn root_without_universe_yaml_still_returned() {
    let tmp = tmp_dir("root-no-yaml");
    let root = tmp.join("work").join("myspace");
    fs::create_dir_all(&root).unwrap();
    // No _universe.yaml — prefix inferred from space name

    let subs = discover_subspaces(&tmp, "myspace");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].key, "myspace");
    assert_eq!(subs[0].prefix, "MYSPACE");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn two_level_nested_subspace_discovered() {
    let tmp = tmp_dir("two-level");
    let root = tmp.join("work").join("yggdrasil");
    let shandara = root.join("shandara");
    fs::create_dir_all(&shandara).unwrap();

    write(
        &root.join("_universe.yaml"),
        "task_prefix: YG\nversion: 1.1.0\n",
    );
    write(
        &shandara.join("_universe.yaml"),
        "task_prefix: SHN\nversion: 0.3.1\nparent: yggdrasil\n",
    );

    let subs = discover_subspaces(&tmp, "yggdrasil");
    assert_eq!(subs.len(), 2, "root + shandara");

    let root_sub = subs.iter().find(|s| s.key == "yggdrasil").unwrap();
    assert_eq!(root_sub.prefix, "YG");
    assert!(root_sub.parent.is_none());

    let sh = subs.iter().find(|s| s.key == "shandara").unwrap();
    assert_eq!(sh.prefix, "SHN");
    assert_eq!(sh.version.as_deref(), Some("0.3.1"));
    assert_eq!(sh.parent.as_deref(), Some("yggdrasil"));
    assert_eq!(sh.abs_path, shandara);
    assert_eq!(sh.rel_path, PathBuf::from("yggdrasil/shandara"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn three_level_all_discovered() {
    let tmp = tmp_dir("three-level");
    let root = tmp.join("work").join("yg");
    let sh = root.join("sh");
    let exp = sh.join("exp");
    fs::create_dir_all(&exp).unwrap();

    write(&root.join("_universe.yaml"), "task_prefix: YG\n");
    write(&sh.join("_universe.yaml"), "task_prefix: SH\n");
    write(&exp.join("_universe.yaml"), "task_prefix: EXP\n");

    let subs = discover_subspaces(&tmp, "yg");
    assert_eq!(subs.len(), 3);
    assert!(subs.iter().any(|s| s.key == "yg" && s.prefix == "YG"));
    assert!(subs.iter().any(|s| s.key == "sh" && s.prefix == "SH"));
    assert!(subs.iter().any(|s| s.key == "exp" && s.prefix == "EXP"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn worktrees_dir_is_skipped() {
    let tmp = tmp_dir("skip-worktrees");
    let root = tmp.join("work").join("yg");
    let worktrees = root.join(".worktrees");
    fs::create_dir_all(&worktrees).unwrap();

    write(&root.join("_universe.yaml"), "task_prefix: YG\n");
    write(&worktrees.join("_universe.yaml"), "task_prefix: WT\n");

    let subs = discover_subspaces(&tmp, "yg");
    assert_eq!(subs.len(), 1, ".worktrees must not be registered");
    assert!(!subs.iter().any(|s| s.prefix == "WT"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn git_dir_is_skipped() {
    let tmp = tmp_dir("skip-git");
    let root = tmp.join("work").join("yg");
    let git = root.join(".git");
    fs::create_dir_all(&git).unwrap();

    write(&root.join("_universe.yaml"), "task_prefix: YG\n");
    write(&git.join("_universe.yaml"), "task_prefix: GIT\n");

    let subs = discover_subspaces(&tmp, "yg");
    assert!(
        !subs.iter().any(|s| s.prefix == "GIT"),
        ".git must be skipped"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn subspace_without_universe_yaml_is_not_registered() {
    let tmp = tmp_dir("no-yaml-child");
    let root = tmp.join("work").join("yg");
    let bare_child = root.join("plain-dir");
    fs::create_dir_all(&bare_child).unwrap();

    write(&root.join("_universe.yaml"), "task_prefix: YG\n");
    // plain-dir has no _universe.yaml

    let subs = discover_subspaces(&tmp, "yg");
    assert_eq!(
        subs.len(),
        1,
        "plain-dir without _universe.yaml should not be registered"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn data_parent_dir_also_discovered() {
    let tmp = tmp_dir("data-parent");
    let root = tmp.join("data").join("myspace");
    fs::create_dir_all(&root).unwrap();
    write(&root.join("_universe.yaml"), "task_prefix: MY\n");

    let subs = discover_subspaces(&tmp, "myspace");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].prefix, "MY");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn prefix_inferred_from_files_when_no_yaml() {
    let tmp = tmp_dir("infer-prefix");
    let root = tmp.join("work").join("proj");
    fs::create_dir_all(&root).unwrap();
    // No _universe.yaml, but MP-1.md exists
    write(&root.join("MP-1.md"), "---\nid: 1\n---\n");

    let subs = discover_subspaces(&tmp, "proj");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].prefix, "MP");

    let _ = fs::remove_dir_all(&tmp);
}
