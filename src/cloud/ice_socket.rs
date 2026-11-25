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
//! and with several m-lines webrtc-rs drops a stream whose SSRC the answer didn't declare.
//! Only packets from the media server count (`Peers`: the UDP candidates of the answers applied): the socket's port is
//! reachable by anyone, and unauthenticated RTP from elsewhere must not plant or replace the SSRCs that get declared.
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

pub struct IceSocket { sock: tokio::net::UdpSocket, video_ssrc: Arc<SeenSsrc>, video_pt: u8, peers: Arc<Peers>, errs: Errs }

/// The addresses the Webex media server sends from: the UDP candidates of the remote descriptions applied
/// (`Media::apply_answer` and the others; host names already resolved). Empty until the first answer: nothing is noted.
#[derive(Debug, Default)]
pub struct Peers(std::sync::RwLock<Vec<SocketAddr>>);

/// At most this many server addresses are kept (ICE restarts add theirs; the oldest go first).
const MAX_PEERS: usize = 64;

impl Peers {
    /// Adds the addresses of a remote description (the call's server may change with an ICE restart).
    pub fn allow(&self, addrs: impl IntoIterator<Item = SocketAddr>) {
        let Ok(mut v) = self.0.write() else { return };
        for a in addrs.into_iter().map(canonical) { if !v.contains(&a) { v.push(a); } }
        let over = v.len().saturating_sub(MAX_PEERS);
        v.drain(..over);
    }
    /// A packet from `from` comes from the media server (address and port).
    pub fn allows(&self, from: SocketAddr) -> bool { self.0.read().is_ok_and(|v| v.contains(&canonical(from))) }
}

/// An IPv4-mapped IPv6 address as plain IPv4, so the two spellings compare equal.
fn canonical(a: SocketAddr) -> SocketAddr { SocketAddr::new(a.ip().to_canonical(), a.port()) }

/// The read errors in a row and when one was last logged (at most once a second).
#[derive(Default)]
struct Errs { row: AtomicU32, logged: std::sync::Mutex<Option<std::time::Instant>> }

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
}

/// A UDP mux on `ip` (an ephemeral port). Needs a running tokio runtime. RTP with payload type `video_pt` from one of
/// `peers` leaves its SSRC in `video_ssrc`.
pub fn mux(ip: IpAddr, video_ssrc: Arc<SeenSsrc>, video_pt: u8, peers: Arc<Peers>) -> std::io::Result<UDPNetwork> {
    let s = std::net::UdpSocket::bind(SocketAddr::new(ip, 0))?;
    s.set_nonblocking(true)?;
    let conn = IceSocket { sock: tokio::net::UdpSocket::from_std(s)?, video_ssrc, video_pt, peers, errs: Errs::default() };
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
    /// A packet arrived from `from` (`None`: the peer of a connected socket, unknown).
    fn note(&self, b: &[u8], from: Option<SocketAddr>) {
        self.errs.row.store(0, Ordering::Relaxed);
        if let Some(s) = rtp_ssrc(b, self.video_pt) { if from.is_some_and(|f| self.peers.allows(f)) { self.video_ssrc.note(s); } }
    }

    /// A read error that is read past: logged at most once a second (the kind only), and after a few in a row the next
    /// read waits 10 ms so a socket that keeps failing doesn't spin.
    async fn skip(&self, e: &std::io::Error) {
        let row = self.errs.row.fetch_add(1, Ordering::Relaxed) + 1;
        let now = std::time::Instant::now();
        let log = self.errs.logged.lock().map(|mut l| { let due = l.is_none_or(|t| now.duration_since(t).as_secs() >= 1); if due { *l = Some(now); } due }).unwrap_or(false);
        if log { super::dbg_conn(&format!("call: ice socket read error skipped ({:?}, {} in a row)", e.kind(), row)); }
        if row >= 5 { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }
}

// The trait is declared with async_trait; these are its expanded signatures.
impl Conn for IceSocket {
    fn connect<'a, 'b>(&'a self, addr: SocketAddr) -> Fut<'b, ()> where 'a: 'b, Self: 'b {
        Box::pin(async move { Ok(self.sock.connect(addr).await?) })
    }
    fn recv<'a, 'c, 'b>(&'a self, buf: &'c mut [u8]) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { loop { match self.sock.recv(buf).await { Err(e) if transient(&e) => self.skip(&e).await, Ok(n) => { self.note(&buf[..n], self.sock.peer_addr().ok()); return Ok(n) } Err(e) => return Err(e.into()) } } })
    }
    fn recv_from<'a, 'c, 'b>(&'a self, buf: &'c mut [u8]) -> Fut<'b, (usize, SocketAddr)> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { loop { match self.sock.recv_from(buf).await { Err(e) if transient(&e) => self.skip(&e).await, Ok((n, a)) => { self.note(&buf[..n], Some(a)); return Ok((n, a)) } Err(e) => return Err(e.into()) } } })
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

    /// RTP from anywhere but the media server's addresses in the answer leaves no SSRC: a stranger who learns the call's
    /// port can't plant or replace the SSRCs that get declared for the far end's camera and share.
    #[test]
    fn only_the_media_server_can_note_an_ssrc() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let seen = Arc::new(SeenSsrc::default());
        let peers = Arc::new(Peers::default());
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_nonblocking(true).unwrap();
        let me = sock.local_addr().unwrap();
        let ice = rt.block_on(async { IceSocket { sock: tokio::net::UdpSocket::from_std(sock).unwrap(), video_ssrc: seen.clone(), video_pt: 102, peers: peers.clone(), errs: Errs::default() } });
        let rtp = |ssrc: u32| { let mut b = [0u8; 12]; b[0] = 0x80; b[1] = 102; b[8..12].copy_from_slice(&ssrc.to_be_bytes()); b };
        let recv = |from: &std::net::UdpSocket, ssrc: u32| {
            from.send_to(&rtp(ssrc), me).unwrap();
            let mut buf = [0u8; 64];
            rt.block_on(ice.recv_from(&mut buf)).unwrap().1
        };
        let (server, stranger) = (std::net::UdpSocket::bind("127.0.0.1:0").unwrap(), std::net::UdpSocket::bind("127.0.0.1:0").unwrap());
        assert_eq!(recv(&server, 7), server.local_addr().unwrap(), "the packet itself is still delivered");
        assert!(!seen.any(), "no answer yet: nothing noted");
        peers.allow([server.local_addr().unwrap()]);
        recv(&stranger, 666);
        assert!(!seen.any(), "a stranger's packet is not noted");
        recv(&server, 7);
        assert_eq!(seen.candidates(&[]), vec![7]);
        recv(&stranger, 666);
        assert_eq!(seen.candidates(&[]), vec![7], "a stranger can't replace it");
        // same address, another port: not the server
        let other_port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        recv(&other_port, 9);
        assert_eq!(seen.candidates(&[]), vec![7]);
        // an IPv4-mapped spelling of the server's address is the same address
        let v4 = server.local_addr().unwrap();
        assert!(peers.allows(SocketAddr::new(match v4.ip() { IpAddr::V4(a) => IpAddr::V6(a.to_ipv6_mapped()), x => x }, v4.port())));
    }

    #[test]
    fn peers_keep_the_newest_addresses() {
        let p = Peers::default();
        let a = |i: u16| SocketAddr::from(([192, 0, 2, 1], 1000 + i));
        p.allow((0..MAX_PEERS as u16 + 3).map(a));
        p.allow([a(5)]);
        assert!(!p.allows(a(0)) && !p.allows(a(2)) && p.allows(a(3)) && p.allows(a(MAX_PEERS as u16 + 2)));
    }

    #[test]
    fn only_a_gone_socket_ends_reading() {
        use std::io::Error as E;
        for k in [ErrorKind::ConnectionReset, ErrorKind::ConnectionRefused, ErrorKind::WouldBlock, ErrorKind::Other] { assert!(transient(&E::from(k)), "{k:?}"); }
        for k in [ErrorKind::NotConnected, ErrorKind::BrokenPipe] { assert!(!transient(&E::from(k)), "{k:?}"); }
        assert!(!transient(&E::from_raw_os_error(if cfg!(windows) { 10038 } else { 9 })));
    }
}
