//! Rewrites a remote description into what webrtc-rs 0.14 accepts, keeping its meaning. webrtc-rs is much stricter
//! than browsers (whose behaviour Webex media servers are built for); each rule below is a check in its parser:
//! * line order: session `v o s [i] c b t a`, media `m c b a` (any other order or line type fails the whole SDP);
//! * `o=` needs six fields with numeric ids, `b=` only CT / AS / TIAS (or `X-`) with a number;
//! * every payload type of `m=` needs an `a=rtpmap` (only 0 is excused);
//! * `a=extmap` ids 1-246 with a URI, else the description fails;
//! * no `a=mid` or no direction: the m-line is silently ignored (no audio);
//! * `a=setup` and the fingerprint algorithm are compared case-sensitively (`passive`, `sha-256`), the fingerprint split
//!   on one space; anything else means a wrong DTLS role or a refused certificate;
//! * a single m-line with `a=ssrc` drops RTP from any other SSRC (media servers send from their own), without it any
//!   SSRC is taken.
use std::io::BufReader;

const SESSION: &str = "vosicbta";
const MEDIA: &str = "mcba";

/// The SDP rewritten for webrtc-rs and the names of the fixes applied (for the log). `mid` is our offer's mid for the
/// first m-line ("" leaves a missing mid alone).
pub fn webrtc_compat(sdp: &str, mid: &str) -> (String, Vec<&'static str>) {
    let mut fixes: Vec<&'static str> = vec![];
    let mut fix = |f: &'static str| if !fixes.contains(&f) { fixes.push(f) };
    let mut sections: Vec<Vec<String>> = vec![vec![]];
    for l in sdp.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let b = l.as_bytes();
        if b.len() < 2 || b[1] != b'=' || !b[0].is_ascii_lowercase() { fix("malformed line dropped"); continue; }
        if b[0] == b'm' { sections.push(vec![]); }
        sections.last_mut().map(|s| s.push(l.to_string()));
    }
    let (mut session, media) = (sections.remove(0), sections);
    // session level
    session.retain(|l| { let ok = SESSION.contains(&l[..1]); if !ok { fix("session line type dropped") } ok });
    if !session.iter().any(|l| l.starts_with("v=")) { session.insert(0, "v=0".into()); fix("v= added") }
    if let Some(o) = session.iter_mut().find(|l| l.starts_with("o=")) {
        let f: Vec<&str> = o[2..].split_whitespace().collect();
        let ok = f.len() == 6 && f[1].parse::<u64>().is_ok() && f[2].parse::<u64>().is_ok() && f[3] == "IN" && matches!(f[4], "IP4" | "IP6");
        if !ok { *o = format!("o={} 0 1 IN IP4 0.0.0.0", f.first().copied().unwrap_or("-")); fix("o= rebuilt") }
    } else { session.push("o=- 0 1 IN IP4 0.0.0.0".into()); fix("o= added") }
    if !session.iter().any(|l| l.starts_with("s=")) { session.push("s=-".into()); fix("s= added") }
    if !session.iter().any(|l| l.starts_with("t=")) { session.push("t=0 0".into()); fix("t= added") }
    let mut out = vec![];
    let mut keep_lines = |lines: Vec<String>, order: &str, fix: &mut dyn FnMut(&'static str)| {
        let kept: Vec<String> = lines.into_iter().filter(|l| {
            if l.starts_with("b=") && !bandwidth_ok(&l[2..]) { fix("b= dropped"); return false; }
            if l.starts_with("a=extmap:") && !extmap_ok(&l[2..]) { fix("bad extmap dropped"); return false; }
            true
        }).collect();
        let mut v: Vec<String> = kept.into_iter().map(|l| norm_attr(l, fix)).collect();
        let pos = |l: &String| order.find(&l[..1]).unwrap_or(order.len());
        let sorted = v.windows(2).all(|w| pos(&w[0]) <= pos(&w[1]));
        if !sorted { v.sort_by_key(pos); fix("lines reordered") } // stable: attribute order stays
        out.extend(v);
    };
    keep_lines(session, SESSION, &mut fix);
    let single = media.len() == 1;
    for (i, mut m) in media.into_iter().enumerate() {
        m.retain(|l| { let ok = MEDIA.contains(&l[..1]); if !ok { fix("media line type dropped") } ok });
        if single { let n = m.len(); m.retain(|l| !l.starts_with("a=ssrc:") && !l.starts_with("a=ssrc-group:")); if m.len() != n { fix("ssrc lines dropped") } }
        if i == 0 && !mid.is_empty() && !m.iter().any(|l| l.starts_with("a=mid:")) { m.push(format!("a=mid:{mid}")); fix("mid added") }
        if !m.iter().any(|l| matches!(l.as_str(), "a=sendrecv" | "a=sendonly" | "a=recvonly" | "a=inactive")) { m.push("a=sendrecv".into()); fix("direction added") }
        let mapped: Vec<String> = m.iter().filter_map(|l| l.strip_prefix("a=rtpmap:")).filter_map(|r| r.split_whitespace().next()).map(String::from).collect();
        let f: Vec<String> = m[0].split_whitespace().map(String::from).collect();
        if f.len() > 3 {
            let pts: Vec<&String> = f[3..].iter().filter(|p| *p == "0" || mapped.contains(p)).collect();
            if !pts.is_empty() && pts.len() != f.len() - 3 {
                m[0] = format!("{} {} {} {}", f[0], f[1], f[2], pts.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "));
                fix("unmapped payload types dropped");
            }
        }
        keep_lines(m, MEDIA, &mut fix);
    }
    // one fingerprint for webrtc-rs: all lines equal, sha-256 preferred
    let fps: Vec<String> = out.iter().filter(|l| l.starts_with("a=fingerprint:")).cloned().collect();
    if let Some(best) = fps.iter().find(|l| l.starts_with("a=fingerprint:sha-256 ")).or(fps.first()).cloned() {
        if fps.iter().any(|l| *l != best) { for l in out.iter_mut().filter(|l| l.starts_with("a=fingerprint:")) { *l = best.clone(); } fix("fingerprints unified") }
    }
    let mut s = out.join("\r\n");
    s.push_str("\r\n");
    (s, fixes)
}

fn bandwidth_ok(v: &str) -> bool {
    let Some((t, n)) = v.split_once(':') else { return false };
    (t.starts_with("X-") || matches!(t, "CT" | "AS" | "TIAS")) && n.parse::<u64>().is_ok()
}

fn extmap_ok(v: &str) -> bool { webrtc::sdp::extmap::ExtMap::unmarshal(&mut BufReader::new(v.as_bytes())).is_ok() }

/// `a=setup` value and fingerprint algorithm in lower case, the fingerprint with single spaces.
fn norm_attr(l: String, fix: &mut dyn FnMut(&'static str)) -> String {
    let n = if let Some(v) = l.strip_prefix("a=setup:") { format!("a=setup:{}", v.trim().to_lowercase()) }
        else if let Some(v) = l.strip_prefix("a=fingerprint:") {
            let mut p = v.split_whitespace();
            format!("a=fingerprint:{} {}", p.next().unwrap_or("").to_lowercase(), p.collect::<Vec<_>>().join(""))
        } else { return l };
    if n != l { fix("setup / fingerprint normalised") }
    n
}

/// The mid of the first m-line of an SDP.
pub fn first_mid(sdp: &str) -> String {
    sdp.lines().map(str::trim).find_map(|l| l.strip_prefix("a=mid:")).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_what_webrtc_rs_refuses() {
        let a = "v=0\r\no=linus 99999999999999999999999 1 IN IP4 media.example.com\r\ns=-\r\nt=0 0\r\nc=IN IP4 192.0.2.10\r\nb=TIAS:64000\r\n\
            a=ice-lite\r\na=fingerprint:SHA-256  AB:CD\r\nx=unknown\r\n  \r\n\
            m=audio 5004 UDP/TLS/RTP/SAVPF 111 101 13\r\nb=RS:0\r\na=rtpmap:111 opus/48000/2\r\na=rtpmap:101 telephone-event/8000\r\n\
            c=IN IP4 192.0.2.10\r\na=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\na=extmap:255 http://protocols.example.com/ts\r\n\
            a=setup:PASSIVE\r\na=fingerprint:sha-1 00:11\r\na=ssrc:1234 cname:x\r\na=ssrc-group:FID 1 2\r\n";
        let (s, fixes) = webrtc_compat(a, "0");
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(&lines[..6], ["v=0", "o=linus 0 1 IN IP4 0.0.0.0", "s=-", "c=IN IP4 192.0.2.10", "b=TIAS:64000", "t=0 0"], "{s}");
        assert!(s.contains("a=fingerprint:sha-256 AB:CD\r\n") && !s.contains("sha-1") && s.matches("a=fingerprint:sha-256 AB:CD").count() == 2, "{s}");
        assert!(s.contains("m=audio 5004 UDP/TLS/RTP/SAVPF 111 101\r\nc=IN IP4 192.0.2.10\r\na=rtpmap:111"), "{s}");
        assert!(s.contains("a=setup:passive") && s.contains("a=mid:0") && s.contains("a=sendrecv") && s.contains("a=extmap:1 "), "{s}");
        assert!(!s.contains("x=") && !s.contains("b=RS") && !s.contains("extmap:255") && !s.contains("a=ssrc"), "{s}");
        assert!(fixes.contains(&"lines reordered") && fixes.contains(&"mid added") && fixes.contains(&"ssrc lines dropped"), "{fixes:?}");
        // the result is a description webrtc-rs parses
        assert!(webrtc::sdp::SessionDescription::unmarshal(&mut std::io::Cursor::new(s.as_bytes())).is_ok());
        assert!(webrtc::sdp::SessionDescription::unmarshal(&mut std::io::Cursor::new(a.as_bytes())).is_err());
    }

    #[test]
    fn leaves_a_clean_answer_alone() {
        let a = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\n\
            a=rtpmap:111 opus/48000/2\r\na=setup:active\r\na=mid:0\r\na=sendrecv\r\n";
        let (s, fixes) = webrtc_compat(a, "0");
        assert_eq!((s.as_str(), fixes.len()), (a, 0));
        assert_eq!(first_mid(a), "0");
    }
}
