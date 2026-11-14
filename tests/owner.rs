//! The `owner.json` claim file: last write wins, read back by every Daemon.

#![cfg(windows)]

use std::path::PathBuf;

use surtr::daemon::OwnerClaim;
use surtr::owner::OwnerFile;

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "surtr-owner-{}-{}-{label}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn a_claim_round_trips_through_the_file() {
    let path = temp_path("round-trip");
    let mut file = OwnerFile::new(Some(path.clone()));

    assert_eq!(file.owner(), None, "no file means no claim");

    file.claim(1234);
    assert_eq!(file.owner(), Some(1234));
    assert_eq!(
        std::fs::read_to_string(&path).ok().as_deref(),
        Some(r#"{"pid":1234}"#),
        "the file is plain JSON the next Daemon can read"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_later_claim_replaces_the_earlier_one() {
    let path = temp_path("handover");
    let mut first = OwnerFile::new(Some(path.clone()));
    let mut second = OwnerFile::new(Some(path.clone()));

    first.claim(100);
    second.claim(200);

    assert_eq!(
        first.owner(),
        Some(200),
        "the first window sees the handover"
    );
    assert_eq!(second.owner(), Some(200));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_claim_creates_its_folder_and_leaves_no_temp_file() {
    let dir = temp_path("nested").with_extension("");
    let path = dir.join("surtr").join("owner.json");
    let mut file = OwnerFile::new(Some(path.clone()));

    file.claim(7);

    assert_eq!(file.owner(), Some(7));
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != "owner.json")
        .collect();
    assert!(leftovers.is_empty(), "the temp file was renamed away");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_corrupt_claim_reads_as_no_claim() {
    let path = temp_path("corrupt");
    std::fs::write(&path, "not json").unwrap();
    let file = OwnerFile::new(Some(path.clone()));

    assert_eq!(file.owner(), None);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn no_path_means_claims_go_nowhere() {
    let mut file = OwnerFile::new(None);
    file.claim(42);

    assert_eq!(file.owner(), None);
}
