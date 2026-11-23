//! Rate limits of the Webex services: after a 429 / 503 a host gets no request until its wait is over. The wait is the
//! answer's `Retry-After` (seconds or an HTTP date), else 2, 4, 8 … s (at most 2 min) with ±25 % jitter. During the wait
//! `Session::http` fails at once without sending anything; the first success clears it.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_WAIT: Duration = Duration::from_secs(120);
/// A server asking for longer is still only waited for this long (a later 429 sets it again).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(600);

/// Per host: until when to wait, and how many 429 / 503 in a row.
#[derive(Default)]
pub struct Backoff(Mutex<HashMap<String, (Instant, u32)>>);

pub static BACKOFF: std::sync::LazyLock<Backoff> = std::sync::LazyLock::new(Backoff::default);

/// The host part of a URL (the unit that is rate limited).
pub fn host_of(url: &str) -> &str { url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("") }

/// Whether a status asks the client to slow down.
pub fn is_throttle(code: u16) -> bool { matches!(code, 429 | 503) }

impl Backoff {
    /// How long `host` must still wait at `now` (`None`: send).
    pub fn wait(&self, host: &str, now: Instant) -> Option<Duration> {
        let m = self.0.lock().ok()?;
        m.get(host).and_then(|(until, _)| until.checked_duration_since(now)).filter(|d| !d.is_zero())
    }
    /// A 429 / 503 from `host`: records the wait and returns it. `jitter` in 0..1.
    pub fn throttled(&self, host: &str, now: Instant, retry_after: Option<Duration>, jitter: f64) -> Duration {
        let Ok(mut m) = self.0.lock() else { return MAX_WAIT };
        let n = m.get(host).map_or(0, |e| e.1) + 1;
        let d = delay(n, retry_after, jitter);
        m.insert(host.to_string(), (now + d, n));
        d
    }
    /// A success from `host`: no more wait.
    pub fn ok(&self, host: &str) { if let Ok(mut m) = self.0.lock() { m.remove(host); } }
}

/// The wait after the `n`-th 429 / 503 in a row: `Retry-After` (never shortened by jitter), else exponential with jitter.
pub fn delay(n: u32, retry_after: Option<Duration>, jitter: f64) -> Duration {
    if let Some(r) = retry_after { return r.min(MAX_RETRY_AFTER).mul_f64(1.0 + 0.1 * jitter.clamp(0.0, 1.0)); }
    let base = Duration::from_secs(1u64 << n.clamp(1, 7)).min(MAX_WAIT);
    base.mul_f64(0.75 + 0.5 * jitter.clamp(0.0, 1.0)).min(MAX_WAIT)
}

/// `Retry-After`: delay seconds, or an IMF-fixdate (`Wed, 21 Oct 2015 07:28:00 GMT`) relative to `now_unix`.
pub fn parse_retry_after(v: &str, now_unix: i64) -> Option<Duration> {
    let v = v.trim();
    if let Ok(s) = v.parse::<u64>() { return Some(Duration::from_secs(s)); }
    let f: Vec<&str> = v.split_whitespace().collect();
    if f.len() != 6 || f[5] != "GMT" { return None; }
    let day: i64 = f[1].parse().ok()?;
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| *m == f[2])? as i64 + 1;
    let year: i64 = f[3].parse().ok()?;
    let t: Vec<i64> = f[4].split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    if t.len() != 3 { return None; }
    // days from civil (Howard Hinnant)
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let at = days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2];
    Some(Duration::from_secs((at - now_unix).max(0) as u64))
}

/// Jitter in 0..1.
pub fn jitter() -> f64 { use rand::Rng; rand::thread_rng().gen::<f64>() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_and_dates() {
        assert_eq!(parse_retry_after("120", 0), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after(" 0 ", 0), Some(Duration::ZERO));
        // 2015-10-21T07:28:00Z = 1445412480
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", 1_445_412_480 - 30), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT", 1_445_412_480 + 99), Some(Duration::ZERO)); // in the past
        assert_eq!(parse_retry_after("Sun, 29 Feb 2004 00:00:01 GMT", 1_078_012_800), Some(Duration::from_secs(1)));
        assert_eq!(parse_retry_after("soon", 0), None);
        assert_eq!(parse_retry_after("Wed, 21 Foo 2015 07:28:00 GMT", 0), None);
    }

    #[test]
    fn backoff_grows_is_capped_and_honours_retry_after() {
        assert_eq!(delay(1, None, 0.5), Duration::from_secs(2));
        assert_eq!(delay(3, None, 0.5), Duration::from_secs(8));
        assert_eq!(delay(30, None, 1.0), MAX_WAIT);
        assert!(delay(2, None, 0.0) >= Duration::from_secs(3) && delay(2, None, 1.0) <= Duration::from_secs(5));
        assert_eq!(delay(5, Some(Duration::from_secs(7)), 0.0), Duration::from_secs(7)); // never shorter than asked
        assert_eq!(delay(1, Some(Duration::from_secs(9999)), 0.0), MAX_RETRY_AFTER);
    }

    #[test]
    fn a_throttled_host_waits_then_recovers() {
        let (b, t0) = (Backoff::default(), Instant::now());
        let h = host_of("https://locus-a.example.com/locus/api/v1/loci/x?y=1");
        assert_eq!(h, "locus-a.example.com");
        assert_eq!(b.wait(h, t0), None);
        assert_eq!(b.throttled(h, t0, Some(Duration::from_secs(5)), 0.0), Duration::from_secs(5));
        assert!(b.wait(h, t0 + Duration::from_secs(4)).is_some() && b.wait("other.example.com", t0).is_none());
        assert_eq!(b.wait(h, t0 + Duration::from_secs(6)), None);
        let second = b.throttled(h, t0 + Duration::from_secs(6), None, 0.5);
        assert_eq!(second, Duration::from_secs(4)); // second in a row: 4 s
        b.ok(h);
        assert_eq!(b.wait(h, t0 + Duration::from_secs(7)), None);
        assert!(is_throttle(429) && is_throttle(503) && !is_throttle(500));
    }
}
