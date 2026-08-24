//! Per-host pacing, shared between processes.
//!
//! Every host that throttles does so by IP, so the budget belongs to the
//! machine and not to a process. A limiter that lives inside one process paces
//! itself impeccably and still goes three times too fast when three of them run
//! at once, which is how a sweep ends up looking hung.
//!
//! So the coordination point is a file, not a transport. Any process that talks
//! to a paced host takes the lock, waits for its turn, stamps the file and
//! releases. A refusal writes its backoff deadline into the same file, so a 429
//! answered to one process slows every other process instead of letting them
//! walk into the same wall.
//!
//! # Bucket file format
//!
//! One file per budget under the bucket directory, named after the budget's key
//! with non-alphanumerics replaced by `_`. The key is usually the host, but a
//! service that throttles one budget across several hostnames maps all of them
//! onto one key, because the file is the budget. Two lines, each an integer,
//! LF-separated:
//!
//! ```text
//! 1786032277123
//! 0
//! ```
//!
//! - line 1: unix milliseconds when the most recent request *returned*
//! - line 2: unix milliseconds until which every caller must hold off, or `0`
//!
//! A missing, empty or unparseable file means no history and is not an error.
//! Line 1 is stamped on completion rather than on dispatch, so a call that
//! spent a minute inside its own retries does not hand the next caller a free
//! pass.
//!
//! This format is deliberately trivial because other languages honour it from
//! the other side. bron owns the definition; changing it breaks them.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Minimum gap between two requests to the same host, when nothing else is
/// known. Hosts with a measured limit override this.
const DEFAULT_INTERVAL: Duration = Duration::from_millis(1_000);

/// Ceiling on any single wait, including one the host asked for. An uncapped
/// `Retry-After` lets one url hold a whole run hostage.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Which budget a request draws from.
///
/// A `&'static str` from a match arm rather than a borrowed host, so a caller
/// cannot pass a hostname where a budget is meant and quietly open a second
/// bucket for a budget that already has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BucketKey(pub &'static str);

/// Pacing for one budget.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// The bucket this host shares. Several hosts may name the same one.
    pub key: BucketKey,
    pub interval: Duration,
}

impl Policy {
    /// The policy for a host.
    ///
    /// web.archive.org is the one measured case: three concurrent sweeps pacing
    /// themselves at 1500 ms each went roughly three times too fast together and
    /// all three stalled. The limit is per IP, so 1500 ms is the machine's
    /// budget, not one process's.
    ///
    /// The Internet Archive answers on two hostnames — web.archive.org for the
    /// index, the replay and Save Page Now, archive.org for availability and
    /// the IAS3 endpoints — and throttles both against the same IP. They name
    /// one bucket, and it is web.archive.org's, because that file already
    /// exists and a PHP process on this machine already opens it. Minting a new
    /// key for the pair would leave the two sides pacing against different
    /// files, which is the failure this module exists to prevent.
    pub fn for_host(host: &str) -> Self {
        Self::for_request(host, "")
    }

    /// The policy for a request, which sometimes needs more than the host.
    ///
    /// Save Page Now answers on the same hostname as the index but is governed
    /// by a concurrent-session quota rather than by the read rate, and it hands
    /// back a hold-off when that quota is tripped. Writing that hold-off into
    /// the read bucket would stall every index query on this machine — this
    /// process and the PHP one both — over a quota neither of them touched.
    /// A refusal must only ever slow the budget it came from.
    pub fn for_request(host: &str, path: &str) -> Self {
        match host {
            "web.archive.org" if path.starts_with("/save") => Self {
                key: BucketKey("web.archive.org/save"),
                interval: Duration::from_millis(1_500),
            },
            "web.archive.org" | "archive.org" => Self {
                key: BucketKey("web.archive.org"),
                interval: Duration::from_millis(1_500),
            },
            _ => Self {
                key: BucketKey(""),
                interval: DEFAULT_INTERVAL,
            },
        }
    }
}

/// A cross-process token bucket, one per budget.
pub struct Bucket {
    path: PathBuf,
    interval: Duration,
}

impl Bucket {
    /// The bucket a host draws from, given its policy.
    ///
    /// An empty key means the host is its own budget, which is the ordinary
    /// case and keeps the filename for every unmeasured host exactly as it was.
    ///
    /// `dir` must already exist: whoever owns the directory creates it once,
    /// so that a bucket does not re-check it on every wakeup of its wait loop.
    pub fn new(dir: &Path, host: &str, policy: Policy) -> Self {
        let BucketKey(key) = policy.key;
        let key = if key.is_empty() { host } else { key };
        Self {
            path: dir.join(format!("{}.bucket", sanitize(key))),
            interval: policy.interval,
        }
    }

    /// Blocks until this process may issue a request to the host.
    ///
    /// Returns how long it waited, which callers report rather than swallow: a
    /// sweep that pauses for two minutes should say so, not look hung.
    pub fn acquire(&self) -> Result<Duration> {
        let started = SystemTime::now();

        loop {
            let wait = {
                let mut file = self.open()?;
                let state = State::read(&mut file)?;
                let wait = state.wait_from(now_millis(), self.interval);

                if wait.is_zero() {
                    // Claim the slot while still holding the lock, so a second
                    // process cannot read the same free slot and take it too.
                    State {
                        last_returned: now_millis(),
                        hold_off_until: 0,
                    }
                    .write(&mut file)?;
                }
                wait
            };

            if wait.is_zero() {
                return Ok(started.elapsed().unwrap_or_default());
            }

            std::thread::sleep(wait.min(MAX_BACKOFF));
        }
    }

    /// Records that a request finished. Call this on completion, not dispatch.
    pub fn released(&self) -> Result<()> {
        let mut file = self.open()?;
        let mut state = State::read(&mut file)?;
        state.last_returned = now_millis();
        state.write(&mut file)
    }

    /// How long to hold off when a host refuses the connection itself.
    ///
    /// A host under load answers 429. A host that has decided about *you*
    /// stops answering: the Internet Archive drops the TCP connection, and a
    /// caller that only recognises 429 reads that as the service being down
    /// and keeps trying. Measured: 171 consecutive requests refused this way
    /// in one run, every one of them reported as "cannot reach", none of them
    /// slowing anything down.
    ///
    /// Long enough that a run stops hammering, short enough that a genuine
    /// blip costs one pause. Not escalating, because that would need a
    /// consecutive-failure count in the bucket file, and the file is a
    /// contract another language reads.
    const UNREACHABLE_BACKOFF: Duration = Duration::from_secs(30);

    /// Records that the host could not be reached at all.
    ///
    /// Backing off here is right whatever the cause: there is no situation in
    /// which continuing to dial a host that is refusing connections is the
    /// correct behaviour.
    pub fn unreachable(&self) -> Result<Duration> {
        self.hold_off(Self::UNREACHABLE_BACKOFF)
    }

    /// Records a refusal, holding every process off until the deadline.
    ///
    /// The wait is never shorter than ordinary pacing: the request that fires
    /// because the host just refused you is the one to be more patient with.
    pub fn refused(&self, retry_after: Option<Duration>) -> Result<Duration> {
        self.hold_off(retry_after.unwrap_or(self.interval))
    }

    fn hold_off(&self, backoff: Duration) -> Result<Duration> {
        let backoff = backoff.clamp(self.interval, MAX_BACKOFF);

        let mut file = self.open()?;
        let mut state = State::read(&mut file)?;
        state.last_returned = now_millis();
        state.hold_off_until = now_millis() + backoff.as_millis() as u64;
        state.write(&mut file)?;

        Ok(backoff)
    }

    fn open(&self) -> Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        lock(&file)?;
        Ok(file)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct State {
    last_returned: u64,
    hold_off_until: u64,
}

impl State {
    fn read(file: &mut File) -> Result<Self> {
        let mut text = String::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_string(&mut text)?;

        // Absent or damaged means no history, which is not an error: the worst
        // case is one unpaced request, and refusing to run is worse.
        let mut lines = text.lines();
        Ok(Self {
            last_returned: lines
                .next()
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(0),
            hold_off_until: lines
                .next()
                .and_then(|l| l.trim().parse().ok())
                .unwrap_or(0),
        })
    }

    fn write(&self, file: &mut File) -> Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        write!(file, "{}\n{}\n", self.last_returned, self.hold_off_until)?;
        file.flush()?;
        Ok(())
    }

    /// How long a caller must wait, given the clock and the pacing interval.
    fn wait_from(&self, now: u64, interval: Duration) -> Duration {
        let until_paced = self
            .last_returned
            .saturating_add(interval.as_millis() as u64);
        let deadline = until_paced.max(self.hold_off_until);
        Duration::from_millis(deadline.saturating_sub(now))
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn sanitize(host: &str) -> String {
    host.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(unix)]
fn lock(file: &File) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    // Advisory, blocking, and released when the descriptor closes, including on
    // a crash. Other languages reach the same lock through flock(2).
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    anyhow::ensure!(result == 0, "cannot lock the rate-limit bucket");
    Ok(())
}

#[cfg(not(unix))]
fn lock(_file: &File) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn policy(ms: u64) -> Policy {
        Policy {
            key: BucketKey(""),
            interval: Duration::from_millis(ms),
        }
    }

    #[test]
    fn a_fresh_bucket_does_not_wait() {
        let dir = tempfile::tempdir().unwrap();
        let bucket = Bucket::new(dir.path(), "example.invalid", policy(10_000));

        // `acquire` reports elapsed time, which on a fresh bucket is the lock
        // and the two file operations rather than a sleep. Measured against an
        // interval of its own size, a cold runner's disk reads as a wait. The
        // interval is ten seconds and the bound one, so the gap between 'did
        // not sleep' and 'slept' is wider than any I/O this test can do.
        let elapsed = bucket.acquire().unwrap();

        assert!(elapsed < Duration::from_secs(1), "elapsed {elapsed:?}");
    }

    #[test]
    fn a_second_caller_waits_for_the_interval() {
        let dir = tempfile::tempdir().unwrap();
        let bucket = Bucket::new(dir.path(), "example.invalid", policy(150));

        bucket.acquire().unwrap();
        bucket.released().unwrap();

        // Timed from the release rather than by what `acquire` reports it
        // slept. The invariant is that no second caller proceeds until the
        // interval has passed since the last request returned, and a runner
        // that stalls between the two has already satisfied it: asserting on
        // the sleep alone reads that correct behaviour as a failure.
        let released_at = Instant::now();
        bucket.acquire().unwrap();
        let since_release = released_at.elapsed();

        // The bucket stamps whole milliseconds, so the moment it recorded can
        // sit up to one before the release actually happened and the observable
        // wait falls that much short. The tolerance is the clock's resolution,
        // not slack: at 149.31ms this failed having found nothing wrong.
        assert!(
            since_release >= Duration::from_millis(149),
            "proceeded {since_release:?} after the release"
        );
    }

    #[test]
    fn the_bucket_file_is_the_documented_two_line_format() {
        let dir = tempfile::tempdir().unwrap();
        let bucket = Bucket::new(dir.path(), "web.archive.org", policy(10));
        bucket.acquire().unwrap();

        let text = std::fs::read_to_string(dir.path().join("web_archive_org.bucket")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        assert!(lines[0].parse::<u64>().unwrap() > 0);
        assert_eq!(lines[1].parse::<u64>().unwrap(), 0);
    }

    #[test]
    fn a_refusal_holds_every_caller_off_and_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let bucket = Bucket::new(dir.path(), "example.invalid", policy(10));

        let backoff = bucket.refused(Some(Duration::from_secs(3_600))).unwrap();
        assert_eq!(
            backoff, MAX_BACKOFF,
            "an uncapped Retry-After holds a run hostage"
        );

        let text = std::fs::read_to_string(dir.path().join("example_invalid.bucket")).unwrap();
        let hold_off: u64 = text.lines().nth(1).unwrap().parse().unwrap();
        assert!(
            hold_off > now_millis(),
            "the deadline is shared, not per process"
        );
    }

    #[test]
    fn a_retry_never_waits_less_than_ordinary_pacing() {
        let dir = tempfile::tempdir().unwrap();
        let bucket = Bucket::new(dir.path(), "example.invalid", policy(2_000));
        let backoff = bucket.refused(Some(Duration::from_millis(1))).unwrap();
        assert_eq!(backoff, Duration::from_millis(2_000));
    }

    #[test]
    fn a_damaged_bucket_file_reads_as_no_history() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("example_invalid.bucket"),
            b"garbage\n\x00\x00",
        )
        .unwrap();

        let bucket = Bucket::new(dir.path(), "example.invalid", policy(10));
        assert!(bucket.acquire().is_ok());
    }

    #[test]
    fn the_archive_host_carries_the_measured_interval() {
        assert_eq!(
            Policy::for_host("web.archive.org").interval,
            Duration::from_millis(1_500)
        );
        assert_eq!(
            Policy::for_host("archive.org").interval,
            Duration::from_millis(1_500)
        );
        assert_eq!(
            Policy::for_host("example.invalid").interval,
            DEFAULT_INTERVAL
        );
    }

    #[test]
    fn both_archive_hosts_draw_from_the_one_bucket_php_already_opens() {
        let dir = tempfile::tempdir().unwrap();

        for host in ["web.archive.org", "archive.org"] {
            Bucket::new(dir.path(), host, Policy::for_host(host))
                .acquire()
                .unwrap();
        }

        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        // One IP, one budget, one file. A second file here means the two hosts
        // pace independently and together go twice too fast.
        assert_eq!(files, ["web_archive_org.bucket"], "{files:?}");
    }

    #[test]
    fn save_page_now_draws_from_its_own_budget() {
        let dir = tempfile::tempdir().unwrap();

        for (host, path) in [
            ("web.archive.org", "/cdx/search/cdx"),
            ("web.archive.org", "/save"),
        ] {
            Bucket::new(dir.path(), host, Policy::for_request(host, path))
                .acquire()
                .unwrap();
        }

        let mut files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();

        // Two budgets, two files. One bucket would let a Save Page Now refusal
        // hold off every index read on the machine, in this process and in the
        // PHP one, over a quota neither of them spent.
        assert_eq!(
            files,
            ["web_archive_org.bucket", "web_archive_org_save.bucket"],
            "{files:?}"
        );
    }

    #[test]
    fn an_unmeasured_host_is_still_its_own_bucket() {
        let dir = tempfile::tempdir().unwrap();
        let host = "example.invalid";
        Bucket::new(dir.path(), host, Policy::for_host(host))
            .acquire()
            .unwrap();

        assert!(dir.path().join("example_invalid.bucket").exists());
    }

    #[test]
    fn wait_is_the_later_of_pacing_and_hold_off() {
        let state = State {
            last_returned: 1_000,
            hold_off_until: 5_000,
        };
        assert_eq!(
            state.wait_from(1_000, Duration::from_millis(100)),
            Duration::from_millis(4_000)
        );

        let state = State {
            last_returned: 1_000,
            hold_off_until: 0,
        };
        assert_eq!(
            state.wait_from(1_000, Duration::from_millis(100)),
            Duration::from_millis(100)
        );
        assert_eq!(
            state.wait_from(2_000, Duration::from_millis(100)),
            Duration::ZERO
        );
    }
}
