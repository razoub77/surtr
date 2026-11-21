//! Console warnings for Lookup Answers that are not definitive: a failed
//! AniList request names its query, and a 429 names the Retry-After delay it
//! will honour. No network, no mpv, no Discord.

use std::cell::Cell;
use std::rc::Rc;

use surtr::anilist::Candidate;
use surtr::daemon::{Level, Policy, Runner};
use surtr::matcher::{Matcher, MediaSource, SourceError};

mod support;

use support::{Host, SharedClaim, PID, T0};

/// Every request answers with the same scripted failure; the count proves the
/// deferred retry still happens.
struct FailingSource {
    error: SourceError,
    requests: Rc<Cell<usize>>,
}

impl FailingSource {
    fn fail(&self) -> SourceError {
        self.requests.set(self.requests.get() + 1);
        self.error
    }
}

impl MediaSource for FailingSource {
    fn search(&mut self, _query: &str) -> Result<Vec<Candidate>, SourceError> {
        Err(self.fail())
    }

    fn sequels(&mut self, _id: i64) -> Result<Vec<Candidate>, SourceError> {
        Err(self.fail())
    }

    fn entry(&mut self, _id: i64) -> Result<Candidate, SourceError> {
        Err(self.fail())
    }
}

fn harness<'m, S: MediaSource + 'm>(
    matcher: &'m mut Matcher<S>,
) -> (Host, Runner<'m, SharedClaim>) {
    let host = Host::new(true);
    let runner = Runner::new(matcher, host.claim(), PID, T0, Policy::default(), Vec::new());
    (host, runner)
}

#[tokio::test]
async fn a_failed_request_warns_with_the_query_and_shows_no_osd_or_presence() {
    let requests = Rc::new(Cell::new(0));
    let mut source = FailingSource {
        error: SourceError::Unavailable,
        requests: requests.clone(),
    };
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Monster\[ASW] Monster - 01.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Warn),
        vec![r#"AniList request failed for query "monster""#],
        "the warning names the query that failed"
    );
    assert!(host.logs().is_empty(), "no Miss line reporting");
    assert!(host.osds().is_empty(), "a failed request is not a Miss");
    assert!(host.publishes().is_empty(), "no Presence");
    assert_eq!(host.clears(), 0, "no Presence was up to clear");
    assert_eq!(requests.get(), 1);
}

#[tokio::test]
async fn a_429_warns_with_the_delay_then_the_spent_retry_stays_silent() {
    let requests = Rc::new(Cell::new(0));
    let mut source = FailingSource {
        error: SourceError::RateLimited(30_000),
        requests: requests.clone(),
    };
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Monster\[ASW] Monster - 01.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Warn),
        vec![r#"AniList throttled query "monster"; retrying in 30s"#],
        "the warning names the Retry-After delay Surtr honours"
    );
    assert!(host.osds().is_empty(), "a throttle is not a Miss");
    assert!(host.publishes().is_empty(), "no Presence before the retry");
    assert_eq!(requests.get(), 1);

    host.advance(30_000);
    host.timer(&mut runner).await;

    assert_eq!(requests.get(), 2, "the deferred retry still happens");
    assert_eq!(
        host.logs_at(Level::Warn).len(),
        1,
        "a spent retry adds no second warning"
    );
    assert!(host.osds().is_empty());
    assert!(host.publishes().is_empty());
}

#[tokio::test]
async fn a_fractional_retry_after_is_reported_in_seconds() {
    let requests = Rc::new(Cell::new(0));
    let mut source = FailingSource {
        error: SourceError::RateLimited(1_500),
        requests,
    };
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Monster\[ASW] Monster - 01.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Warn),
        vec![r#"AniList throttled query "monster"; retrying in 1.5s"#],
        "the delay is seconds, not milliseconds"
    );
}
