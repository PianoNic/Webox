//! The call's one ICE socket, owned here instead of by webrtc-ice: UDP, bound only to the given local address, shared by
//! all candidates through webrtc-ice's UDP mux.
//!
//! Why: on Windows a datagram answered with ICMP "port unreachable" (a media server port that is closed or rejected by a
//! firewall, e.g. one of the server's two ports) makes the next `recv_from` fail with WSAECONNRESET. webrtc-ice (0.14
//! and 0.17) then stops reading that socket for good, so ICE never completes even though the other port works. Here
//! such reads are skipped.
use std::future::Future;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use webrtc::ice::udp_mux::{UDPMuxDefault, UDPMuxParams};
use webrtc::ice::udp_network::UDPNetwork;
use webrtc::util::{Conn, Error};

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

pub struct IceSocket(tokio::net::UdpSocket);

/// A UDP mux on `ip` (an ephemeral port). Needs a running tokio runtime.
pub fn mux(ip: IpAddr) -> std::io::Result<UDPNetwork> {
    let s = std::net::UdpSocket::bind(SocketAddr::new(ip, 0))?;
    s.set_nonblocking(true)?;
    let conn = IceSocket(tokio::net::UdpSocket::from_std(s)?);
    Ok(UDPNetwork::Muxed(UDPMuxDefault::new(UDPMuxParams::new(conn))))
}

/// The error a datagram's ICMP reply leaves on the socket (Windows); nothing is wrong with the socket itself.
fn transient(e: &std::io::Error) -> bool { e.kind() == ErrorKind::ConnectionReset }

// The trait is declared with async_trait; these are its expanded signatures.
impl Conn for IceSocket {
    fn connect<'a, 'b>(&'a self, addr: SocketAddr) -> Fut<'b, ()> where 'a: 'b, Self: 'b {
        Box::pin(async move { Ok(self.0.connect(addr).await?) })
    }
    fn recv<'a, 'c, 'b>(&'a self, buf: &'c mut [u8]) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { loop { match self.0.recv(buf).await { Err(e) if transient(&e) => continue, r => return Ok(r?) } } })
    }
    fn recv_from<'a, 'c, 'b>(&'a self, buf: &'c mut [u8]) -> Fut<'b, (usize, SocketAddr)> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { loop { match self.0.recv_from(buf).await { Err(e) if transient(&e) => continue, r => return Ok(r?) } } })
    }
    fn send<'a, 'c, 'b>(&'a self, buf: &'c [u8]) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { Ok(self.0.send(buf).await?) })
    }
    fn send_to<'a, 'c, 'b>(&'a self, buf: &'c [u8], target: SocketAddr) -> Fut<'b, usize> where 'a: 'b, 'c: 'b, Self: 'b {
        Box::pin(async move { Ok(self.0.send_to(buf, target).await?) })
    }
    fn local_addr(&self) -> Result<SocketAddr, Error> { Ok(self.0.local_addr()?) }
    fn remote_addr(&self) -> Option<SocketAddr> { self.0.peer_addr().ok() }
    fn close<'a, 'b>(&'a self) -> Fut<'b, ()> where 'a: 'b, Self: 'b { Box::pin(async { Ok(()) }) }
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) { self }
}
