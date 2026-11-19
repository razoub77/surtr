//! `overrides.json` loading: silent when absent, normalized keys, and a bad
//! file or bad entry that never takes the valid ones down with it.

use std::path::{Path, PathBuf};

use surtr::overrides::Overrides;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "surtr-overrides-{}-{}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

/// A unique path under the OS temp folder; the file is never created.
fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "surtr-overrides-{}-{}-{label}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn a_missing_file_is_silent_and_empty() {
    let (overrides, messages) = Overrides::load(&temp_path("missing"));

    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), None);
}

#[test]
fn an_empty_object_is_silent_and_empty() {
    let (overrides, messages) = Overrides::parse("{}");

    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), None);
}

#[test]
fn keys_are_normalized_for_case_and_whitespace() {
    let (overrides, messages) = Overrides::parse(
        r#"{
            "HONZUKI NO GEKOKUJOU": 108268,
            "  Youjo   Senki   Season 2 ": 135865
        }"#,
    );

    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert_eq!(overrides.id("Honzuki   No Gekokujou"), Some(108268));
    assert_eq!(overrides.id("youjo senki season 2"), Some(135865));
    assert_eq!(overrides.id("Youjo Senki Season 2"), Some(135865));
}

#[test]
fn invalid_json_is_one_line_and_matching_runs_normally() {
    let (overrides, messages) = Overrides::parse("{ not json");

    assert_eq!(messages.len(), 1, "got: {messages:?}");
    assert!(messages[0].contains("overrides.json"), "got: {}", messages[0]);
    assert!(messages[0].contains("invalid JSON"), "got: {}", messages[0]);
    assert_eq!(overrides.id("honzuki no gekokujou"), None);
}

#[test]
fn a_non_object_top_level_is_one_line_and_ignored() {
    let (overrides, messages) = Overrides::parse(r#"["honzuki no gekokujou", 108268]"#);

    assert_eq!(messages.len(), 1, "got: {messages:?}");
    assert!(messages[0].contains("object"), "got: {}", messages[0]);
    assert_eq!(overrides.id("honzuki no gekokujou"), None);
}

#[test]
fn a_malformed_entry_is_ignored_without_breaking_the_valid_ones() {
    let (overrides, messages) = Overrides::parse(
        r#"{
            "honzuki no gekokujou": 108268,
            "youjo senki season 2": "135865",
            "zero": 0,
            "negative": -1,
            "fractional": 1.5
        }"#,
    );

    assert_eq!(messages.len(), 4, "got: {messages:?}");
    for key in ["youjo senki season 2", "zero", "negative", "fractional"] {
        assert!(
            messages.iter().any(|message| message.contains(key)),
            "no message names {key:?}: {messages:?}"
        );
    }
    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert_eq!(overrides.id("youjo senki season 2"), None);
}

#[test]
fn valid_legacy_overrides_move_to_the_target_byte_for_byte() {
    let dir = TempDir::new("move");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    let text = "{\r\n  \"Honzuki No Gekokujou\": 108268\r\n}\r\n";
    write(&legacy, text);

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert_eq!(messages.len(), 1, "got: {messages:?}");
    assert!(messages[0].contains("moved overrides"), "got: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), text);
    assert!(!legacy.exists(), "legacy file must be removed after the move");
}

#[test]
fn invalid_legacy_overrides_are_left_alone_and_create_nothing() {
    let dir = TempDir::new("invalid-legacy");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, "{ not json");

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert!(!target.exists(), "nothing may be created from invalid legacy data");
    assert!(legacy.exists(), "the only copy of the data must survive");
    assert!(
        messages.iter().any(|message| message.contains("invalid JSON")),
        "got: {messages:?}"
    );
    assert_eq!(overrides.id("honzuki no gekokujou"), None);
}

#[test]
fn a_valid_legacy_file_is_merged_into_an_existing_target() {
    let dir = TempDir::new("merge");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, r#"{"youjo senki season 2": 135865}"#);
    write(&target, r#"{"honzuki no gekokujou": 108268}"#);

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert_eq!(overrides.id("youjo senki season 2"), Some(135865));
    assert!(!legacy.exists(), "legacy file must be removed after the merge");
    let merged: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(merged.as_object().unwrap().len(), 2);
}

#[test]
fn a_conflicting_target_id_wins_and_is_named_on_the_console() {
    let dir = TempDir::new("conflict");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, r#"{"honzuki no gekokujou": 111111}"#);
    write(&target, r#"{"honzuki no gekokujou": 108268}"#);

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("honzuki no gekokujou") && message.contains("108268")),
        "got: {messages:?}"
    );
    assert!(!legacy.exists());
}

#[test]
fn merge_compares_normalized_keys_and_skips_the_same_value() {
    let dir = TempDir::new("normalized");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, r#"{"  HONZUKI   NO GEKOKUJOU ": 108268}"#);
    write(&target, r#"{"honzuki no gekokujou": 108268}"#);

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert!(messages.is_empty(), "same key + same value is not a conflict: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert!(!legacy.exists());
    let merged: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(merged.as_object().unwrap().len(), 1);
}

#[test]
fn an_invalid_target_blocks_the_merge_and_keeps_both_files() {
    let dir = TempDir::new("invalid-target");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, r#"{"youjo senki season 2": 135865}"#);
    write(&target, "{ broken");

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert_eq!(messages.len(), 1, "got: {messages:?}");
    assert!(
        messages[0].contains("invalid JSON")
            && messages[0].contains("surtr-overrides.json")
            && messages[0].contains("files left unchanged"),
        "got: {messages:?}"
    );
    assert!(target.exists());
    assert!(legacy.exists(), "legacy must survive a broken target");
    assert_eq!(overrides.id("youjo senki season 2"), None);
}

#[test]
fn an_invalid_legacy_file_does_not_stop_the_valid_target_loading() {
    let dir = TempDir::new("invalid-legacy-with-target");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    let target_text = "{\"honzuki no gekokujou\": 108268}\n";
    write(&legacy, "{ broken");
    write(&target, target_text);

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert!(!messages.is_empty(), "the broken legacy file must be reported");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), target_text);
    assert!(legacy.exists());
}

#[test]
fn a_failed_move_falls_back_to_reading_the_legacy_file() {
    let dir = TempDir::new("failed-move");
    let legacy = dir.file("overrides.json");
    let target = dir.file("missing").join("surtr-overrides.json");
    write(&legacy, r#"{"honzuki no gekokujou": 108268}"#);

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert_eq!(overrides.id("honzuki no gekokujou"), Some(108268));
    assert!(!messages.is_empty(), "the failed move must be reported");
    assert!(legacy.exists(), "the source must survive a failed move");
    assert!(!target.exists());
}

#[test]
fn duplicate_normalized_keys_in_the_legacy_file_keep_the_last_value() {
    let dir = TempDir::new("duplicates");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, r#"{"Anime A": 1, "anime a": 2}"#);
    write(&target, "{}");

    let (overrides, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(overrides.id("anime a"), Some(2));
    assert!(!legacy.exists());
}

#[test]
fn a_target_pointing_at_the_legacy_file_is_never_deleted() {
    let dir = TempDir::new("same-path");
    let path = dir.file("overrides.json");
    write(&path, r#"{"honzuki no gekokujou": 108268}"#);

    let (first, messages) = Overrides::load_with_migration(Some(&path), Some(&path));
    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(first.id("honzuki no gekokujou"), Some(108268));
    assert!(path.exists());

    let (second, messages) = Overrides::load_with_migration(Some(&path), Some(&path));
    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(second.id("honzuki no gekokujou"), Some(108268));
}

#[test]
fn two_broken_files_name_both_failures() {
    let dir = TempDir::new("both-broken");
    let legacy = dir.file("overrides.json");
    let target = dir.file("surtr-overrides.json");
    write(&legacy, "{ bad legacy");
    write(&target, "{ bad target");

    let (_, messages) = Overrides::load_with_migration(Some(&target), Some(&legacy));

    assert_eq!(messages.len(), 1, "got: {messages:?}");
    assert!(
        messages[0].contains("surtr-overrides.json")
            && messages[0].contains("overrides.json")
            && messages[0].contains("files left unchanged"),
        "got: {messages:?}"
    );
    assert!(legacy.exists());
    assert!(target.exists());
}

#[test]
fn without_a_target_overrides_are_disabled_and_the_legacy_file_is_untouched() {
    let dir = TempDir::new("no-target");
    let legacy = dir.file("overrides.json");
    write(&legacy, r#"{"honzuki no gekokujou": 108268}"#);

    let (overrides, messages) = Overrides::load_with_migration(None, Some(&legacy));

    assert!(messages.is_empty(), "got: {messages:?}");
    assert_eq!(overrides.id("honzuki no gekokujou"), None);
    assert!(legacy.exists());
}
