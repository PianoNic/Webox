//! SDP helpers for the media leg: what webrtc-rs can take from a Webex answer, and log summaries without addresses.
//!
//! Webex media servers list UDP, TCP and TLS (`xTLS`, port 443) candidates. webrtc-rs only does UDP ICE, and it rejects
//! the whole description on a transport it doesn't know, so the answer keeps only its UDP candidates.
//! Their addresses may be host names (FQDN); webrtc-rs only takes IP literals, so those are resolved first (`resolve_names`).
use std::collections::HashMap;
use std::net::IpAddr;

/// What a candidate / connection address is, for the log: `ip4`, `ip6`, `mdns` or `fqdn`.
fn addr_kind(a: &str) -> &'static str {
    match a.parse::<IpAddr>() { Ok(IpAddr::V4(_)) => "ip4", Ok(IpAddr::V6(_)) => "ip6", Err(_) if a.ends_with(".local") => "mdns", Err(_) => "fqdn" }
}

/// Outcome of `resolve_names`, for the log (counts only).
#[derive(Debug, Default, PartialEq)]
pub struct Resolved { pub names: usize, pub candidates: usize, pub unresolved: usize }

/// Replaces host names in `a=candidate:` and `c=` lines by addresses from `lookup` (IPv4 only unless `v6`): one candidate
/// per address, with a unique foundation and a slightly lower priority for each extra one. A candidate whose name
/// doesn't resolve is dropped; an unresolvable `c=` line gets 0.0.0.0 (`::` for IPv6).
pub fn resolve_names(sdp: &str, v6: bool, lookup: &dyn Fn(&str) -> Vec<IpAddr>) -> (String, Resolved) {
    let mut cache: HashMap<String, Vec<IpAddr>> = HashMap::new();
    let mut get = |name: &str| cache.entry(name.to_string()).or_insert_with(|| lookup(name).into_iter().filter(|ip| ip.is_ipv4() || v6).collect()).clone();
    let (mut out, mut r) = (String::with_capacity(sdp.len()), Resolved::default());
    for line in sdp.split_inclusive('\n') {
        let (body, eol) = line.find(['\r', '\n']).map_or((line, ""), |i| (&line[..i], &line[i..]));
        if let Some(c) = body.strip_prefix("a=candidate:") {
            let f: Vec<&str> = c.split_whitespace().collect();
            if f.len() >= 8 && addr_kind(f[4]) == "fqdn" {
                r.names += 1;
                let ips = get(f[4]);
                if ips.is_empty() { r.unresolved += 1; }
                let prio: u32 = f[3].parse().unwrap_or(0);
                for (i, ip) in ips.iter().enumerate() {
                    let mut g: Vec<String> = f.iter().map(|s| s.to_string()).collect();
                    if i > 0 { g[0] = format!("{}r{i}", f[0]); g[3] = prio.saturating_sub(i as u32).to_string(); }
                    g[4] = ip.to_string();
                    out.push_str(&format!("a=candidate:{}{eol}", g.join(" ")));
                    r.candidates += 1;
                }
                continue;
            }
        } else if let Some(c) = body.strip_prefix("c=") {
            let f: Vec<&str> = c.split_whitespace().collect();
            if f.len() == 3 && addr_kind(f[2]) == "fqdn" {
                let ip = get(f[2]).first().map_or(if v6 { "::".into() } else { "0.0.0.0".into() }, |i| i.to_string());
                let fam = if ip.contains(':') { "IP6" } else { "IP4" };
                out.push_str(&format!("c={} {fam} {ip}{eol}", f[0]));
                continue;
            }
        }
        out.push_str(line);
    }
    (out, r)
}

/// The first `a=candidate:` line webrtc-rs can't read, as "#index (transport/type, address kind): error". No addresses.
pub fn first_bad_candidate(sdp: &str) -> Option<String> {
    use webrtc::ice::candidate::candidate_base::unmarshal_candidate;
    sdp.lines().filter_map(|l| l.trim().strip_prefix("a=candidate:")).enumerate().find_map(|(i, c)| {
        let e = unmarshal_candidate(c).err()?;
        let f: Vec<&str> = c.split_whitespace().collect();
        let typ = f.iter().position(|x| *x == "typ").and_then(|i| f.get(i + 1)).copied().unwrap_or("?");
        Some(format!("candidate #{i} ({}/{typ}, {}): {e}", f.get(2).unwrap_or(&"?").to_lowercase(), f.get(4).map_or("?", |a| addr_kind(a))))
    })
}

/// Transport of an `a=candidate:` line (lower case), `None` for other lines.
fn transport(line: &str) -> Option<String> {
    let c = line.trim_end().strip_prefix("a=candidate:")?;
    c.split_whitespace().nth(2).map(str::to_lowercase)
}

/// The SDP with only the candidates webrtc-rs can use (UDP), and the transports of those it dropped.
pub fn udp_only(sdp: &str) -> (String, Vec<String>) {
    let (mut out, mut dropped) = (String::with_capacity(sdp.len()), vec![]);
    for line in sdp.split_inclusive('\n') {
        match transport(line) {
            Some(t) if t != "udp" => dropped.push(t),
            _ => out.push_str(line),
        }
    }
    (out, dropped)
}

/// One log line about an SDP: candidates by transport / type / port, ICE lite, DTLS setup, m-lines. No addresses.
pub fn summary(sdp: &str) -> String {
    let mut cands: Vec<String> = vec![];
    let (mut lite, mut setup, mut mlines, mut conn) = (false, String::new(), vec![], vec![]);
    for l in sdp.lines().map(str::trim) {
        if let Some(c) = l.strip_prefix("a=candidate:") {
            let f: Vec<&str> = c.split_whitespace().collect();
            let typ = f.iter().position(|x| *x == "typ").and_then(|i| f.get(i + 1)).copied().unwrap_or("?");
            let tcp = f.iter().position(|x| *x == "tcptype").and_then(|i| f.get(i + 1)).map_or(String::new(), |t| format!("/{t}"));
            cands.push(format!("{}/{typ}{tcp}:{}#{}@{}", f.get(2).unwrap_or(&"?").to_lowercase(), f.get(5).unwrap_or(&"?"), f.get(1).unwrap_or(&"?"), f.get(4).map_or("?", |a| addr_kind(a))));
        } else if l == "a=ice-lite" {
            lite = true;
        } else if let Some(s) = l.strip_prefix("a=setup:") {
            setup = s.to_string();
        } else if let Some(m) = l.strip_prefix("m=") {
            mlines.push(m.split_whitespace().take(3).collect::<Vec<_>>().join(" "));
        } else if let Some(c) = l.strip_prefix("c=") {
            conn.push(c.split_whitespace().nth(2).map_or("?", addr_kind));
        }
    }
    format!("m=[{}] c=[{}] ice-lite={lite} setup={setup} candidates=[{}]", mlines.join("; "), conn.join(" "), cands.join(" "))
}

/// The addresses of the UDP candidates (IP literals only: `resolve_names` ran before): where the media server sends from.
pub fn udp_candidate_addrs(sdp: &str) -> Vec<std::net::SocketAddr> {
    sdp.lines().filter_map(|l| l.trim().strip_prefix("a=candidate:")).filter_map(|c| {
        let f: Vec<&str> = c.split_whitespace().collect();
        if !f.get(2)?.eq_ignore_ascii_case("udp") { return None }
        Some(std::net::SocketAddr::new(f.get(4)?.parse().ok()?, f.get(5)?.parse().ok()?))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_are_the_udp_candidates() {
        let a = udp_candidate_addrs(ANSWER);
        assert_eq!(a, ["192.0.2.10:5004".parse().unwrap(), "192.0.2.10:9000".parse().unwrap()]);
        assert!(udp_candidate_addrs("a=candidate:1 1 UDP 1 media.example.com 5004 typ host\r\n").is_empty(), "names are resolved first");
    }

    /// The shape of a Webex media server answer (documentation addresses).
    const ANSWER: &str = "v=0\r\no=linus 0 1 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\na=ice-lite\r\n\
        m=audio 5004 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=setup:passive\r\n\
        a=candidate:1 1 UDP 2130706431 192.0.2.10 5004 typ host\r\n\
        a=candidate:2 1 UDP 2130706175 192.0.2.10 9000 typ host\r\n\
        a=candidate:3 1 TCP 1962934271 192.0.2.10 5004 typ host tcptype passive\r\n\
        a=candidate:4 1 xTLS 1962933759 192.0.2.10 443 typ host tcptype passive\r\na=rtcp-mux\r\n";

    #[test]
    fn keeps_only_udp_candidates() {
        let (s, dropped) = udp_only(ANSWER);
        assert_eq!(dropped, ["tcp", "xtls"]);
        assert_eq!(s.matches("a=candidate:").count(), 2);
        assert!(s.contains("a=rtcp-mux\r\n") && s.contains("a=ice-lite\r\n") && s.ends_with("\r\n"));
        assert_eq!(udp_only(&s).1.len(), 0);
    }

    #[test]
    fn summary_has_no_addresses() {
        let s = summary(ANSWER);
        assert!(!s.contains("192.0.2"), "{s}");
        assert!(s.contains("ice-lite=true setup=passive"), "{s}");
        assert!(s.contains("udp/host:5004#1") && s.contains("xtls/host/passive:443#1"), "{s}");
        assert!(s.contains("m=[audio 5004 UDP/TLS/RTP/SAVPF]"), "{s}");
    }

    /// An answer naming its media server by host name, resolved by a stub (no DNS).
    const FQDN: &str = "v=0\r\nc=IN IP4 media.example.com\r\na=ice-lite\r\nm=audio 5004 UDP/TLS/RTP/SAVPF 111\r\n\
        a=candidate:1 1 UDP 2130706431 media.example.com 5004 typ host\r\n\
        a=candidate:2 1 UDP 2130706175 media.example.com 9000 typ host\r\n\
        a=candidate:3 1 UDP 2130706175 gone.example.net 9000 typ host\r\n\
        a=candidate:4 1 UDP 2130706000 192.0.2.7 5004 typ host\r\na=rtcp-mux\r\n";
    fn stub(name: &str) -> Vec<IpAddr> {
        match name { "media.example.com" => vec!["192.0.2.10".parse().unwrap(), "2001:db8::1".parse().unwrap(), "192.0.2.11".parse().unwrap()], _ => vec![] }
    }

    #[test]
    fn host_names_become_addresses() {
        assert!(first_bad_candidate(FQDN).unwrap().starts_with("candidate #0 (udp/host, fqdn): parse addr"), "{:?}", first_bad_candidate(FQDN));
        let (s, r) = resolve_names(FQDN, false, &stub);
        assert_eq!(r, Resolved { names: 3, candidates: 4, unresolved: 1 });
        assert!(s.contains("c=IN IP4 192.0.2.10\r\n") && !s.contains("example") && !s.contains("2001:db8"), "{s}");
        assert!(s.contains("a=candidate:1 1 UDP 2130706431 192.0.2.10 5004 typ host\r\n"), "{s}");
        assert!(s.contains("a=candidate:1r1 1 UDP 2130706430 192.0.2.11 5004 typ host\r\n"), "{s}");
        assert!(s.contains("a=candidate:2r1 1 UDP 2130706174 192.0.2.11 9000 typ host\r\n"), "{s}");
        assert!(s.contains("a=candidate:4 1 UDP 2130706000 192.0.2.7 5004 typ host\r\n") && s.ends_with("a=rtcp-mux\r\n"));
        assert_eq!(first_bad_candidate(&s), None);
        let (v6, _) = resolve_names(FQDN, true, &stub);
        assert!(v6.contains(" 2001:db8::1 5004 typ host"));
        // names only in the log kinds, never the names themselves
        let sum = summary(FQDN);
        assert!(sum.contains("c=[fqdn]") && sum.contains("udp/host:5004#1@fqdn") && sum.contains("@ip4") && !sum.contains("example"), "{sum}");
        // a c= line nobody can resolve still parses
        let (s, _) = resolve_names("c=IN IP4 gone.example.net\r\n", false, &stub);
        assert_eq!(s, "c=IN IP4 0.0.0.0\r\n");
    }

    /// webrtc-rs itself: an answer with Webex' TLS candidate is refused as a whole, the filtered one is taken.
    #[test]
    fn webrtc_refuses_xtls_but_takes_the_filtered_answer() {
        use webrtc::ice::candidate::candidate_base::unmarshal_candidate;
        assert!(unmarshal_candidate("4 1 xTLS 1962933759 192.0.2.10 443 typ host tcptype passive").is_err());
        assert!(unmarshal_candidate("1 1 UDP 2130706431 192.0.2.10 5004 typ host").is_ok());
    }
}
