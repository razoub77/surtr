//! The Daemon's command line contract with `main.lua`: configuration parsing
//! and the help text the binary prints.

use surtr::cli::{anime_roots, client_id, pause_timeout_ms, title_language};
use surtr::matcher::TitleLanguage;

#[test]
fn anime_roots_are_split_on_semicolons_and_trimmed() {
    assert_eq!(
        anime_roots(Some(r"D:\Anime; E:\Anime2")),
        vec![r"D:\Anime".to_owned(), r"E:\Anime2".to_owned()]
    );
}

#[test]
fn empty_anime_root_segments_are_ignored() {
    // A blank conf, a trailing separator, doubled separators and a list of
    // nothing but separators all mean "no roots", never a root called "".
    assert!(anime_roots(None).is_empty());
    assert!(anime_roots(Some("")).is_empty());
    assert!(anime_roots(Some(" ; ; ")).is_empty());
    assert_eq!(
        anime_roots(Some(r"D:\Anime;; E:\Anime2 ;")),
        vec![r"D:\Anime".to_owned(), r"E:\Anime2".to_owned()]
    );
}

#[test]
fn anime_roots_keep_inner_spaces() {
    // Trailing path spaces are stripped by Windows itself, but spaces inside
    // a folder name are part of it.
    assert_eq!(
        anime_roots(Some(r"  D:\My Anime  ;E:\Other  ")),
        vec![r"D:\My Anime".to_owned(), r"E:\Other".to_owned()]
    );
}

#[test]
fn client_id_is_trimmed_or_absent() {
    // mp.options keeps whitespace around a value; the flag form must not
    // inherit it (`--client-id " 123 "` is the same id as `123`).
    assert_eq!(client_id(Some("  1234567890123456789 ")), Some("1234567890123456789".to_owned()));
    assert_eq!(client_id(Some("1234567890123456789")), Some("1234567890123456789".to_owned()));
}

#[test]
fn a_blank_client_id_means_unset() {
    // A conf line left empty (`client_id=`), a space-only value and a missing
    // flag all mean "fall back to SURTR_CLIENT_ID", never an empty id.
    assert_eq!(client_id(None), None);
    assert_eq!(client_id(Some("")), None);
    assert_eq!(client_id(Some("   ")), None);
}

#[test]
fn the_title_language_is_matched_case_insensitively() {
    // mp.options passes the conf value verbatim, casing included.
    assert_eq!(title_language(Some("romaji")), TitleLanguage::Romaji);
    assert_eq!(title_language(Some("  ROMAJI ")), TitleLanguage::Romaji);
    assert_eq!(title_language(Some("English")), TitleLanguage::English);
}

#[test]
fn an_unrecognized_title_language_keeps_the_default() {
    // A blank conf line, a missing flag, or a typo is never worth refusing
    // startup over: the built-in default, english, stands in.
    assert_eq!(title_language(None), TitleLanguage::English);
    assert_eq!(title_language(Some("")), TitleLanguage::English);
    assert_eq!(title_language(Some("romai")), TitleLanguage::English);
}

#[test]
fn pause_timeout_is_whole_seconds_with_zero_disabled() {
    assert_eq!(pause_timeout_ms(None), Ok(0));
    assert_eq!(pause_timeout_ms(Some("  ")), Ok(0));
    assert_eq!(pause_timeout_ms(Some("0")), Ok(0));
    assert_eq!(pause_timeout_ms(Some(" 300 ")), Ok(300_000));
}

#[test]
fn a_bad_pause_timeout_falls_back_to_disabled() {
    assert!(pause_timeout_ms(Some("abc")).is_err());
    assert!(pause_timeout_ms(Some("-1")).is_err());
    assert!(pause_timeout_ms(Some("18446744073709551615")).is_err());
}
