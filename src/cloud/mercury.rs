use super::*;

/// First wait before re-opening a dropped socket (ms).
pub const RECONNECT_FIRST_MS: u64 = 3000;
/// Later waits double up to this cap (ms).
pub const RECONNECT_MAX_MS: u64 = 32_000;
/// A ping that isn't answered within this closes the socket ("Pong not received"). Without it a dead TCP connection
/// (sleep, network change) would stay "connected" forever and no event would arrive.
pub const PONG_TIMEOUT_SECS: u64 = 14;

/// The socket URL: the device's `webSocketUrl` plus only `clientTimestamp` and `mercuryRegistrationStatus`
/// (no `outboundWireFormat` / `bufferStates` / `aliasHttpStatus`).
pub fn socket_url(ws: &str, now_ms: u128) -> String {
    let sep = if ws.contains('?') { '&' } else { '?' };
    format!("{ws}{sep}clientTimestamp={now_ms}&mercuryRegistrationStatus=true")
}

/// What to do after the socket closed with `code` / `reason`: `false` = stay offline. 4000 "replaced" = another socket of
/// this device took over; 4001 on the active socket and 1003 are permanent. Every other close is reconnected.
pub fn reconnect_after_close(code: u16, reason: &str) -> bool {
    !matches!((code, reason.to_ascii_lowercase().as_str()), (4000, "replaced") | (4001, _) | (1003, _))
}

/// The real-time socket: authorises, acknowledges every event, forwards key-service answers and activities, pings on a fixed
/// schedule with a pong timeout and reconnects with back-off. Ends when the session that started it is replaced (sign-out,
/// new sign-in) so two sockets of one device never fight each other (which the service answers with close 4000 "replaced").
pub(super) fn mercury(ws: String, gen: u64, jobs: Sender<Job>, kms: Sender<String>, out: Sender<Resp>) {
    use tungstenite::{protocol::frame::coding::CloseCode, stream::MaybeTlsStream, Message as M};
    let mut wait_ms = RECONNECT_FIRST_MS;
    let mut down_since: Option<i64> = None; // when the last socket that was up went down (Unix ms), for the catch-up
    loop {
        if !session_current(gen) { return }
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let conn = tungstenite::connect(socket_url(&ws, ms).as_str());
        let mut close: Option<(u16, String)> = None;
        match conn {
            Err(e) => dbg_conn(&format!("mercury connect error: {}", e.to_string().chars().take(120).collect::<String>())),
            Ok((mut sock, resp)) => {
                dbg_conn(&format!("mercury connected, http {}", resp.status()));
                // Short reads so the ping / pong timers and the session check run on time.
                let t = Some(Duration::from_millis(500));
                match sock.get_ref() {
                    MaybeTlsStream::Plain(s) => { let _ = s.set_read_timeout(t); }
                    MaybeTlsStream::Rustls(s) => { let _ = s.get_ref().set_read_timeout(t); }
                    _ => {}
                }
                // The bearer of the moment: renewed in place by `Req::Token`, so a reconnect never uses an expired one.
                let auth = json!({"id": jose::uuid(), "type": "authorization", "data": {"token": format!("Bearer {}", live_token())}, "trackingId": format!("CLIENT_{}", jose::uuid())});
                if sock.send(M::Text(auth.to_string())).is_ok() { let _ = out.send(Resp::Status("Connected (live)".into())); }
                let mut last_pong = Instant::now();
                let mut ping: Option<(String, Instant)> = None;
                let mut online = false;
                loop {
                    if !session_current(gen) { let _ = sock.close(None); let _ = sock.flush(); return }
                    match sock.read() {
                        Ok(M::Text(t)) => if handle(&t, &mut sock, &jobs, &kms, &mut ping, &mut last_pong, &mut online, down_since) == Flow::Stop { return },
                        Ok(M::Binary(b)) => if let Ok(t) = String::from_utf8(b.to_vec()) {
                            if handle(&t, &mut sock, &jobs, &kms, &mut ping, &mut last_pong, &mut online, down_since) == Flow::Stop { return }
                        },
                        Ok(M::Close(f)) => { close = f.map(|f| (u16::from(f.code), f.reason.to_string())); dbg_conn(&format!("mercury closed by the service: {close:?}")); }
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                        Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => break,
                        Err(e) => { dbg_conn(&format!("mercury read error: {e}")); break }
                    }
                    if online { wait_ms = RECONNECT_FIRST_MS; } // reset the back-off once mercury is connected
                    // Fixed ping schedule: a ping MERCURY_PING_SECS after the last pong, closed when its pong doesn't come.
                    match &ping {
                        Some((_, sent)) if sent.elapsed() > Duration::from_secs(PONG_TIMEOUT_SECS) => {
                            dbg_conn("mercury: pong not received, closing");
                            let _ = sock.close(Some(tungstenite::protocol::CloseFrame { code: CloseCode::Normal, reason: "Pong not received".into() }));
                            let _ = sock.flush();
                            break;
                        }
                        None if last_pong.elapsed() >= Duration::from_secs(MERCURY_PING_SECS) => {
                            let id = jose::uuid();
                            if sock.send(M::Text(json!({"id": id, "type": "ping"}).to_string())).is_err() { break }
                            ping = Some((id, Instant::now()));
                        }
                        _ => {}
                    }
                }
                // This socket was up: the next one catches up from here.
                if online { down_since = Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)); }
            }
        }
        if let Some((code, reason)) = &close {
            if !reconnect_after_close(*code, reason) {
                dbg_conn(&format!("mercury: closed with {code}, not reconnecting (another socket of this device took over, or permanent)"));
                let _ = out.send(Resp::Status("Offline".into()));
                return;
            }
        }
        let _ = out.send(Resp::Status("Reconnecting…".into()));
        // Wait in small steps so a replaced session ends promptly.
        let until = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < until { if !session_current(gen) { return } std::thread::sleep(Duration::from_millis(250)); }
        wait_ms = (wait_ms * 2).min(RECONNECT_MAX_MS);
    }
}

#[derive(PartialEq)]
enum Flow { Go, Stop }

/// One text frame: acknowledge it (every message with an id), match pongs to the outstanding ping, and route the event.
#[allow(clippy::too_many_arguments)]
fn handle<S: std::io::Read + std::io::Write>(t: &str, sock: &mut tungstenite::WebSocket<S>, jobs: &Sender<Job>, kms: &Sender<String>,
    ping: &mut Option<(String, Instant)>, last_pong: &mut Instant, online: &mut bool, down_since: Option<i64>) -> Flow {
    use tungstenite::Message as M;
    let Ok(v) = serde_json::from_str::<Value>(t) else { return Flow::Go };
    if let Some(id) = v["id"].as_str() { let _ = sock.send(M::Text(json!({"messageId": id, "type": "ack"}).to_string())); }
    if v["type"] == "pong" {
        // A pong for another ping closes the socket ("Pong mismatch").
        if ping.as_ref().is_some_and(|(id, _)| v["id"].as_str().is_some_and(|x| x != id)) { dbg_conn("mercury: pong mismatch"); }
        *ping = None;
        *last_pong = Instant::now();
        return Flow::Go;
    }
    let event = v["data"]["eventType"].as_str().unwrap_or("");
    dbg_conn(&format!("mercury msg type={} event={event}", v["type"]));
    match event {
        // The socket counts as up once one of these arrives.
        "mercury.buffer_state" | "mercury.registration_status" => {
            if let (false, Some(since)) = (*online, down_since) {
                // After a reconnect, catch up on what was missed (conversation catch-up, a fresh presence subscription).
                let _ = jobs.send(Job::Catchup(since));
            }
            *online = true;
        }
        "encryption.kms_message" => for m in v["data"]["encryption"]["kmsMessages"].as_array().into_iter().flatten() {
            if let Some(s) = m.as_str() { let _ = kms.send(s.to_string()); }
        },
        "conversation.activity" => {
            // The typing registration's lifetime rides on every activity (cloud/typing.rs).
            if let Some(ttl) = v["data"]["conversation"]["registrationTTLMs"].as_i64() {
                let conv = v["data"]["activity"]["target"]["id"].as_str().unwrap_or("").to_string();
                if !conv.is_empty() { let _ = jobs.send(Job::TypingTtl(conv, ttl)); }
            }
            if jobs.send(Job::Activity(v["data"]["activity"].clone())).is_err() { return Flow::Stop }
        }
        // Someone typing (cloud/typing.rs).
        "status.start_typing" | "status.stop_typing" => { let _ = jobs.send(Job::Typing(v["data"].clone())); }
        e if e.starts_with("locus.") => { let _ = jobs.send(Job::Locus(v["data"].clone())); }
        "directory.search" => { let _ = jobs.send(Job::Directory(v["data"].clone())); } // people search results
        // A toggle changed for this user.
        "featureToggle_update" => { if features::live_update(&v["data"]["featureToggle"]) { dbg_conn("feature toggle updated live"); } }
        e if e.starts_with("apheleia") || e.starts_with("usersub") || e.contains("presence") => { let _ = jobs.send(Job::Presence(v["data"].clone())); }
        // A calendar meeting was created / changed / deleted: re-read the calendar (cloud/polls.rs).
        e if polls::calendar_event(e) => { let _ = jobs.send(Job::Ui(Req::Calendar)); }
        // janus.user_sessions / user_recent_sessions / user_sessions_deleted: re-read the history.
        e if e.starts_with("janus.user") => { let _ = jobs.send(Job::Ui(Req::CallHistory)); }
        _ => {}
    }
    // The first event of any kind also means the socket is up (the registration status may not come on every server).
    if !*online && !event.is_empty() { if let Some(since) = down_since { let _ = jobs.send(Job::Catchup(since)); } *online = true; }
    Flow::Go
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_url_is_the_originals() {
        assert_eq!(socket_url("wss://m/v1/apps/wx2/registrations/r/messages", 5), "wss://m/v1/apps/wx2/registrations/r/messages?clientTimestamp=5&mercuryRegistrationStatus=true");
        assert_eq!(socket_url("wss://m/x?a=1", 5), "wss://m/x?a=1&clientTimestamp=5&mercuryRegistrationStatus=true");
    }
    #[test]
    fn close_codes_like_the_sdk() {
        assert!(!reconnect_after_close(4000, "replaced"));
        assert!(!reconnect_after_close(4000, "Replaced"));
        assert!(reconnect_after_close(4000, "other"));
        assert!(!reconnect_after_close(4001, ""));
        assert!(!reconnect_after_close(1003, "rejected"));
        for c in [1000, 1001, 1005, 1006, 1011, 1008] { assert!(reconnect_after_close(c, "idle")); }
    }
    #[test]
    fn backoff() {
        let mut w = RECONNECT_FIRST_MS; let mut seen = vec![];
        for _ in 0..6 { seen.push(w); w = (w * 2).min(RECONNECT_MAX_MS); }
        assert_eq!(seen, [3000, 6000, 12000, 24000, 32000, 32000]);
    }
}
