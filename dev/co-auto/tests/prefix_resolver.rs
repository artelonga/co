use co_auto::resolve_task_id;
use std::path::Path;

#[test]
fn full_key_passed_through_as_uppercase() {
    let wd = Path::new("/tmp");
    assert_eq!(resolve_task_id("CO-272", "co", wd), "CO-272");
    assert_eq!(resolve_task_id("co-272", "co", wd), "CO-272");
}

#[test]
fn bare_number_with_known_spaces() {
    let wd = Path::new("/tmp");
    assert_eq!(resolve_task_id("272", "co", wd), "CO-272");
    assert_eq!(resolve_task_id("50", "yggdrasil", wd), "YG-50");
    assert_eq!(resolve_task_id("10", "rfq", wd), "RFQ-10");
    assert_eq!(resolve_task_id("5", "qb", wd), "QB-5");
    assert_eq!(resolve_task_id("1", "artelonga", wd), "AL-1");
}

#[test]
fn unknown_space_uses_uppercase_fallback() {
    let wd = Path::new("/tmp");
    assert_eq!(resolve_task_id("42", "myspace", wd), "MYSPACE-42");
}

#[test]
fn universe_yaml_task_prefix_overrides_table() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let tmp = std::env::temp_dir().join(format!("co-auto-prefix-{}", ts));
    let space_dir = tmp.join("work").join("testspace");
    std::fs::create_dir_all(&space_dir).unwrap();
    std::fs::write(space_dir.join("_universe.yaml"), "task_prefix: XYZ\n").unwrap();
    let result = resolve_task_id("10", "testspace", &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(result, "XYZ-10");
}

#[test]
fn file_scan_infers_prefix_when_no_yaml_or_table_match() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let tmp = std::env::temp_dir().join(format!("co-auto-scan-{}", ts));
    let space_dir = tmp.join("work").join("myproj");
    std::fs::create_dir_all(&space_dir).unwrap();
    std::fs::write(space_dir.join("MP-1.md"), "---\nid: 1\n---\n").unwrap();
    let result = resolve_task_id("5", "myproj", &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(result, "MP-5");
}
