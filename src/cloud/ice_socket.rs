//! The call's one ICE socket, owned here instead of by webrtc-ice: UDP, bound only to the given local address, shared by
//! all candidates through webrtc-ice's UDP mux.
//!
//! Why: on Windows a datagram answered with ICMP "port unreachable" (a media server port that is closed or rejected by a
//! firewall, e.g. one of the server's two ports) makes the next `recv_from` fail with WSAECONNRESET. webrtc-ice (0.14
//! and 0.17) then stops reading that socket for good, so ICE never completes even though the other port works. Here
//! such reads are skipped.
//!
//! It also notes the SSRCs that H.264 (the video lines' payload type) arrives from (`SeenSsrc`, the last two different
//! ones: the far end's camera on `video/main` and its share on `video/slides`): the RTP header is not encrypted by SRTP,
//! and with several m-lines webrtc-rs drops a stream whose SSRC the answer didn't declare. The same for Opus (the
//! audio lines' payload type): the call offers all four m-lines from the start, so the server's audio is taken only once
//! its SSRC is declared (`Media::redeclare`).
use std::future::Future;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use webrtc::ice::udp_mux::{UDPMuxDefault, UDPMuxParams};
use webrtc::ice::udp_network::UDPNetwork;
use webrtc::util::{Conn, Error};

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

pub struct IceSocket { sock: tokio::net::UdpSocket, seen: Seen, errs: Errs }

/// What the socket notes about arriving RTP: the SSRCs of the video lines' payload type and of the audio lines' (Opus).
#[derive(Clone)]
pub struct Seen { pub video: Arc<SeenSsrc>, pub video_pt: u8, pub audio: Arc<SeenSsrc>, pub audio_pt: u8 }

/// The read errors in a row and when one was last logged (at most once a second).
#[derive(Default)]
struct Errs { row: AtomicU32, logged: std::sync::Mutex<Option<std::time::Instant>>, since: std::sync::Mutex<Option<std::time::Instant>> }

/// A read error is logged at most this often (the call log is always on).
const ERR_LOG_EVERY: std::time::Duration = std::time::Duration::from_secs(10);
/// Errors in a row for this long (the network is gone): the socket is read only once a second until a datagram arrives.
const ERR_BACKOFF_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// How long to wait before the next read after `row` errors in a row, for `for_` (10 ms after a few, 1 s after 30 s).
pub fn err_pause(row: u32, for_: std::time::Duration) -> Option<std::time::Duration> {
    if for_ >= ERR_BACKOFF_AFTER { Some(std::time::Duration::from_secs(1)) } else if row >= 5 { Some(std::time::Duration::from_millis(10)) } else { None }
}

/// The last two different SSRCs a payload type arrived from (0 = none). Two streams arriving interleaved leave both.
#[derive(Debug, Default)]
pub struct SeenSsrc { last: AtomicU32, prev: AtomicU32 }

impl SeenSsrc {
    pub fn note(&self, s: u32) {
        let last = self.last.load(Ordering::Relaxed);
        if last != s { self.prev.store(last, Ordering::Relaxed); self.last.store(s, Ordering::Relaxed); }
    }
    /// The SSRCs seen, newest first, without `known` ones (streams webrtc-rs already takes).
    pub fn candidates(&self, known: &[u32]) -> Vec<u32> {
        let mut v = vec![];
        for s in [self.last.load(Ordering::Relaxed), self.prev.load(Ordering::Relaxed)] { if s != 0 && !known.contains(&s) && !v.contains(&s) { v.push(s); } }
        v
    }
    /// Anything seen at all.
    pub fn any(&self) -> bool { self.last.load(Ordering::Relaxed) != 0 }
    /// The SSRC seen last (0 = none).
    pub fn newest(&self) -> u32 { self.last.load(Ordering::Relaxed) }
}

/// A UDP mux on `ip` (an ephemeral port; the unspecified address = every interface). Needs a running tokio runtime.
/// RTP of the video and audio payload types leaves its SSRC in `seen`.
pub fn mux(ip: IpAddr, seen: Seen) -> std::io::Result<UDPNetwork> {
    let s = std::net::UdpSocket::bind(SocketAddr::new(ip, 0))?;
    s.set_nonblocking(true)?;
    let conn = IceSocket { sock: tokio::net::UdpSocket::from_std(s)?, seen, errs: Errs::default() };
    Ok(UDPNetwork::Muxed(UDPMuxDefault::new(UDPMuxParams::new(conn))))
}

/// Whether a read error means the socket itself is gone; every other one (the ICMP reply a datagram leaves on Windows,
/// a network that went away for a moment, a full buffer) is read past: webrtc-ice never reads a socket again after an error.
fn transient(e: &std::io::Error) -> bool {
    const GONE: i32 = if cfg!(windows) { 10038 } else { 9 }; // WSAENOTSOCK / EBADF
    !matches!(e.kind(), ErrorKind::NotConnected | ErrorKind::BrokenPipe) && e.raw_os_error() != Some(GONE)
}

/// The SSRC of an RTP packet with payload type `pt` (RFC 7983: first byte 128..191; RTCP's types 200..206 excluded).
pub fn rtp_ssrc(b: &[u8], pt: u8) -> Option<u32> {
    if b.len() < 12 || !(128..=191).contains(&b[0]) || (192..=223).contains(&b[1]) || b[1] & 0x7f != pt { return None; }
    Some(u32::from_be_bytes([b[8], b[9], b[10], b[11]]))
}

impl IceSocket {
    fn note(&self, b: &[u8]) {
        if self.errs.row.swap(0, Ordering::Relaxed) > 0 { if let Ok(mut s) = self.errs.since.lock() { *s = None; } }
        if let Some(s) = rtp_ssrc(b, self.seen.video_pt) { self.seen.video.note(s); } else if let Some(s) = rtp_ssrc(b, self.seen.audio_pt) { self.seen.audio.note(s); }
    }

    /// A read error that is read past: logged at most every `ERR_LOG_EVERY` (the kind only); after a few in a row the next
    /// read waits 10 ms so a socket that keeps failing doesn't spin, and after `ERR_BACKOFF_AFTER` of nothing but errors
    /// (the network is gone) it waits 1 s. Reading never stops: webrtc-ice would never read the socket again, and the media
    /// could not recover once the network is back.
    async fn skip(&self, e: &std::io::Error) {
        let row = self.errs.row.fetch_add(1, Ordering::Relaxed) + 1;
        let now = std::time::Instant::now();
        let since = self.errs.since.lock().map(|mut s| *s.get_or_insert(now)).unwrap_or(now);
        let log = self.errs.logged.lock().map(|mut l| { let due = l.is_none_or(|t| now.duration_since(t) >= ERR_LOG_EVERY); if due { *l = Some(now); } due }).unwrap_or(false);
        if log { super::dbg_conn(&format!("call: ice socket read error skipped ({:?}, {} in a row for {} s)", e.kind(), row, now.duration_since(since).as_secs())); }
        if let Some(p) = err_pause(row, now.duration_since(since)) { tokio::time::sleep(p).await; }
    }
}

// The trait is declared with async_trait; these are its expanded signatures.
impl Conn for IceSocket {
    fn connect<'a, 'b>(&'a self, addr: SocketAddr) -> Fut<'b, ()> where 'a: 'b, Self: 'b {
        Box::pin(async move { Ok(self.sock.connect(addr).await?) })
    }
    fn recv<'a, 'c, 'b>(&'a self, buf: &'c mut [u8]) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { loop { match self.sock.recv(buf).await { Err(e) if transient(&e) => self.skip(&e).await, Ok(n) => { self.note(&buf[..n]); return Ok(n) } Err(e) => return Err(e.into()) } } })
    }
    fn recv_from<'a, 'c, 'b>(&'a self, buf: &'c mut [u8]) -> Fut<'b, (usize, SocketAddr)> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { loop { match self.sock.recv_from(buf).await { Err(e) if transient(&e) => self.skip(&e).await, Ok((n, a)) => { self.note(&buf[..n]); return Ok((n, a)) } Err(e) => return Err(e.into()) } } })
    }
    fn send<'a, 'c, 'b>(&'a self, buf: &'c [u8]) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { Ok(self.sock.send(buf).await?) })
    }
    fn send_to<'a, 'c, 'b>(&'a self, buf: &'c [u8], target: SocketAddr) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { Ok(self.sock.send_to(buf, target).await?) })
    }
    fn local_addr(&self) -> Result<SocketAddr, Error> { Ok(self.sock.local_addr()?) }
    fn remote_addr(&self) -> Option<SocketAddr> { self.sock.peer_addr().ok() }
    fn close<'a, 'b>(&'a self) -> Fut<'b, ()> where 'a: 'b, Self: 'b { Box::pin(async { Ok(()) }) }
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) { self }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rtp_ssrc_only_for_rtp_of_that_type() {
        let mut rtp = [0u8; 12];
        rtp[0] = 0x80; rtp[1] = 0x80 | 102; rtp[8..12].copy_from_slice(&0x0102_0304u32.to_be_bytes());
        assert_eq!(rtp_ssrc(&rtp, 102), Some(0x0102_0304), "marker bit set, PT 102");
        rtp[1] = 102;
        assert_eq!(rtp_ssrc(&rtp, 102), Some(0x0102_0304));
        assert_eq!(rtp_ssrc(&rtp, 111), None, "another payload type");
        rtp[1] = 200; // RTCP sender report
        assert_eq!(rtp_ssrc(&rtp, 200 & 0x7f), None);
        assert_eq!(rtp_ssrc(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 102), None, "STUN");
        assert_eq!(rtp_ssrc(&[0x80, 102], 102), None, "too short");
    }

    #[test]
    fn two_interleaved_video_streams_are_both_seen() {
        let s = SeenSsrc::default();
        assert!(!s.any() && s.candidates(&[]).is_empty());
        for x in [7, 7, 9, 7, 9, 9] { s.note(x); }
        assert_eq!(s.candidates(&[]), vec![9, 7], "newest first");
        assert_eq!(s.candidates(&[9]), vec![7], "the stream webrtc-rs takes already is no candidate");
        s.note(5);
        assert_eq!(s.candidates(&[]), vec![5, 9]);
    }

    #[test]
    fn read_errors_back_off_hard_after_30_s() {
        use std::time::Duration as D;
        assert_eq!(err_pause(1, D::from_secs(0)), None, "one error: read again at once");
        assert_eq!(err_pause(5, D::from_secs(1)), Some(D::from_millis(10)), "a few in a row: no spinning");
        assert_eq!(err_pause(5000, D::from_secs(30)), Some(D::from_secs(1)), "the network is gone: once a second");
    }

    #[test]
    fn only_a_gone_socket_ends_reading() {
        use std::io::Error as E;
        for k in [ErrorKind::ConnectionReset, ErrorKind::ConnectionRefused, ErrorKind::WouldBlock, ErrorKind::Other] { assert!(transient(&E::from(k)), "{k:?}"); }
        for k in [ErrorKind::NotConnected, ErrorKind::BrokenPipe] { assert!(!transient(&E::from(k)), "{k:?}"); }
        assert!(!transient(&E::from_raw_os_error(if cfg!(windows) { 10038 } else { 9 })));
    }
}
