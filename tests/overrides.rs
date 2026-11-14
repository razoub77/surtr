//! `overrides.json` loading: silent when absent, normalized keys, and a bad
//! file or bad entry that never takes the valid ones down with it.

use std::path::PathBuf;

use surtr::overrides::Overrides;

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
