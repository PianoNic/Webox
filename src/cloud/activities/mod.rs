//! Messages in a space and what can be done to them, all as conversation-service activities with
//! the text encrypted with the space's key: post, reply in a thread (parent type "reply"), edit
//! (parent type "edit"), delete, react (object type "reaction2", parent type "reaction") and read
//! receipts (verb "acknowledge").
use super::*;
use std::collections::HashSet;

/// Reaction names Webex uses, with the emoji shown for each.
pub const REACTIONS: [(&str, &str); 13] = [
    ("thumbsup", "👍"), ("heart", "❤"), ("smiley", "😊"), ("haha", "😂"), ("wow", "😮"), ("sad", "😢"), ("celebrate", "🎉"),
    ("thumbsdown", "👎"), ("confused", "😕"), ("prayer", "🙏"), ("clap", "👏"), ("fire", "🔥"), ("raisehand", "✋"),
];

/// `object` of a posted comment: `displayName` always; `content` + `mentions` only when the message has markup / mentions
/// (a plain message has neither in real data). `enc` encrypts one string with the space's key.
pub(super) fn comment_object(r: &crate::richtext::Rendered, typed: &str, enc: &mut dyn FnMut(&str) -> Result<String, String>) -> Result<Value, String> {
    let plain = if r.plain.is_empty() { typed } else { r.plain.as_str() }; // the service rejects `content` without a displayName
    let mut o = json!({"objectType": "comment", "displayName": enc(plain)?});
    if r.html.is_empty() { return Ok(o); }
    o["content"] = json!(enc(&r.html)?);
    let people: Vec<Value> = r.mentions.iter().filter(|m| !m.all).map(|m| json!({"id": m.id, "objectType": "person"})).collect();
    if !people.is_empty() { o["mentions"] = json!({"items": people}); }
    // @all: a sibling key `groupMentions` (items[{groupType "all", objectType "groupMention"}]), not part of `mentions`.
    if r.mentions.iter().any(|m| m.all) {
        o["groupMentions"] = json!({"items": [{"groupType": "all", "objectType": "groupMention"}]});
    }
    Ok(o)
}

/// UTC now as "YYYY-MM-DDTHH:MM:SS.mmmZ" (the `aad` of an scr is such a timestamp).
pub(super) fn iso_now() -> String { // pub(super): file SCRs use the same aad
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468; let era = z.div_euclid(146_097); let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

/// The `share` activity for a Giphy GIF, in the shape real Giphy shares have: `object {objectType "content",
/// contentCategory "images", files.items[{objectType "file", id, displayName (JWE), encryptionKeyUrl, fileSize,
/// mimeType "image/gif", url "https://giphy.com", scr (JWE of {aad, enc, iv, key, loc}), image {width, height, url, scr}}]}`.
/// The `scr` key/iv are random placeholders (the file isn't encrypted); `loc` is the media URL. The still (`image.scr`) uses the
/// same URL (real shares point it at a smaller rendition; we only have one).
#[allow(clippy::too_many_arguments)]
pub(super) fn gif_share_body(key_url: &str, title: &str, url: &str, w: u32, h: u32, size: u64, file_id: &str, now_iso: &str,
    enc_text: &mut dyn FnMut(&str) -> Result<String, String>, enc_scr: &mut dyn FnMut(&Value) -> Result<String, String>) -> Result<Value, String> {
    use rand::RngCore;
    let mut scr = |loc: &str| -> Result<String, String> {
        let (mut key, mut iv) = ([0u8; 32], [0u8; 12]);
        rand::thread_rng().fill_bytes(&mut key); rand::thread_rng().fill_bytes(&mut iv);
        enc_scr(&json!({"aad": now_iso, "enc": "A256GCM", "iv": jose::b64(&iv), "key": jose::b64(&key), "loc": loc}))
    };
    let name = if title.trim().is_empty() { "giphy.gif" } else { title.trim() };
    let file = json!({"objectType": "file", "id": file_id, "displayName": enc_text(name)?, "encryptionKeyUrl": key_url, "fileSize": size,
        "mimeType": "image/gif", "url": "https://giphy.com", "scr": scr(url)?,
        "image": {"width": w, "height": h, "url": "https://giphy.com", "scr": scr(url)?}});
    Ok(json!({"verb": "share", "encryptionKeyUrl": key_url,
        "object": {"objectType": "content", "contentCategory": "images", "files": {"items": [file]}}}))
}

/// Endpoint of a message with attachments (a GIF share is one): objectType "content", verb "share". A message without
/// attachments goes to "activities". `personRefresh` is not sent here.
pub(super) const GIF_SHARE_PATH: &str = "/content?transcode=true&async=false";

/// Activities per page when stepping back in time (Webex loads history in blocks of 20). `older_messages` repeats the
/// request until 20 new visible rows arrived or the start is reached.
pub const HISTORY_PAGE: u32 = 20;
/// At most this many requests per step (a step that needs more is ended early).
pub const HISTORY_MAX_REQUESTS: u32 = 10;

/// Per space: every raw activity loaded so far (newest page + older pages), whether the start of the chat was reached, and
/// the id of the first message handed to the UI (to count what an older page prepended).
#[derive(Default)]
pub(super) struct History {
    pub acts: HashMap<String, Vec<Value>>,
    pub done: HashSet<String>,
    pub first: HashMap<String, String>,
    /// The service's answer to the last activity I posted (`submit`, `send_gif`) and the clientTempId it carried; taken by
    /// `after_write`.
    pub posted: Option<(Value, String)>,
    /// Activities I posted whose live (mercury) event hasn't arrived yet; overdue ones make the space reload (`echo_overdue`).
    pub echo: Vec<Echo>,
    /// Decrypted texts by ciphertext, so folding a space again (after a send, a reaction, a live edit) doesn't decrypt every
    /// message once more.
    pub plain: HashMap<String, String>,
    /// Reactors' (name, e-mail) by uuid, read from their reaction activities (`reactor_names`).
    pub names: HashMap<String, (String, String)>,
}

/// A posted activity waiting for its live event: matched by activity id or by the clientTempId it was sent with.
#[derive(Clone, Debug)]
pub(super) struct Echo { pub space: String, pub id: String, pub temp: String, pub due: std::time::Instant }

/// How long the live event of my own activity may take before the space is reloaded instead (normally it arrives a few
/// ms after the POST answer and is just de-duplicated).
pub(super) const ECHO_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a read receipt is due: the readable activity is newer than what the space has as seen (ISO times compare as text).
/// Unknown `published` → no receipt.
pub(super) fn ack_due(last_seen: &str, published: &str) -> bool { !published.is_empty() && published > last_seen }

/// The page before the oldest loaded activity: `maxDate = oldestPublishedDate - 1` in epoch ms, so the boundary activity
/// is not sent again (the ISO date or the plain ms value would re-send it).
pub(super) fn older_query(space: &str, oldest_iso: &str) -> String {
    let ms = super::people::iso_ms(oldest_iso);
    let max = if ms > 0 { (ms - 1).to_string() } else { oldest_iso.to_string() };
    format!("/conversations/{space}/activities?limit={HISTORY_PAGE}&maxDate={max}{}", activity_flags())
}

/// Query flags appended to every activity request: `&personRefresh=true` (see `activity_flags`), then always
/// `&uuidEntryFormat=true&includeChildren=true`. With them, actor / parent / reaction-user ids come as uuids
/// (actors keep displayName and emailAddress).
pub(super) const ACTIVITY_FLAGS: &str = "&personRefresh=true&uuidEntryFormat=true&includeChildren=true";

/// `ACTIVITY_FLAGS` for this account: `&personRefresh=true` is left out when the feature toggle
/// `desktop-last-modified-ci-contact-refresh-enabled` reads true.
pub(super) fn activity_flags() -> &'static str {
    let skip = super::features::feature("desktop-last-modified-ci-contact-refresh-enabled").and_then(|v| v.as_bool()) == Some(true);
    if skip { &ACTIVITY_FLAGS["&personRefresh=true".len()..] } else { ACTIVITY_FLAGS }
}

/// One add / leave info row as the join/leave collapsing sees it.
pub(super) struct JoinLeave { pub add: bool, pub own: bool, pub actor: String, pub actor_name: String, pub actor_me: bool, pub person: Person }

/// Join/leave collapsing. Adjacent add / leave rows of the same day merge when (A) the same actor added - or removed -
/// several people, or (B) several people joined - or left - themselves; any other row in between ends the run; a single
/// row stays as it is. The merged row lists the people (`Message.group`) and is drawn as a collapsed row (link colour,
/// arrow, popover), worded with the "{{0}} added {{1}} people to this space." string family. "Day" is the UTC day of the row.
pub(super) fn collapse_join_leave(list: Vec<Message>, joins: &HashMap<String, JoinLeave>) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(list.len());
    let mut run: Option<(usize, String)> = None; // (index in out of the run's first row, its id)
    for m in list {
        let Some(j) = joins.get(&m.id) else { out.push(m); run = None; continue };
        if let Some((at, first)) = &run {
            let g = &joins[first];
            let fits = out[*at].date == m.date && g.add == j.add && g.own == j.own && (g.own || (!g.actor.is_empty() && g.actor == j.actor));
            if fits {
                let row = &mut out[*at];
                if row.group.is_empty() { row.group.push(g.person.clone()); }
                row.group.push(j.person.clone());
                let n = row.group.len();
                row.notice = match (g.own, g.add, g.actor_me) {
                    (false, true, true) => crate::trq!("You added {{0}} people to this space.", n),
                    (false, true, false) => crate::trq!("{{0}} added {{1}} people to this space.", g.actor_name, n),
                    (false, false, true) => crate::trq!("You removed {{0}} people from this space.", n),
                    (false, false, false) => crate::trq!("{{0}} removed {{1}} people from this space.", g.actor_name, n),
                    (true, true, _) => crate::trq!("{{0}} people joined this space.", n),
                    (true, false, _) => crate::trq!("{{0}} people left this space.", n),
                };
                continue;
            }
        }
        run = Some((out.len(), m.id.clone()));
        out.push(m);
    }
    out
}

/// The newest page: `<conversationUrl>/activities?limit=N&maxDate=<now + 7 days>` (604 800 000 ms ahead).
pub(super) fn newest_query(space: &str, now_ms: i64) -> String {
    format!("/conversations/{space}/activities?limit={ACTIVITY_PAGE}&maxDate={}{}", now_ms + 604_800_000, activity_flags())
}

pub(super) fn catchup_query(since_ms: i64) -> String { format!("/conversations/catchup?sinceDate={since_ms}") }

fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

/// A call's length as written in "You had a call ({{0}})": "{{0}} sec", "{{0}} min", "{{0}} hr {{1}} min".
/// Whole minutes, rounded down.
pub(super) fn call_duration(secs: u64) -> String {
    match secs {
        0..=59 => crate::trq!("{{0}} sec", secs),
        60..=3599 => crate::trq!("{{0}} min", secs / 60),
        _ => crate::trq!("{{0}} hr {{1}} min", secs / 3600, secs % 3600 / 60),
    }
}

/// Which file the chat list shows for a shared image: `(url, scr, fallback still scr)`. Animated GIFs (mime, `.gif` name or
/// a thumbnail block saying gif) use the file itself so they animate in the list; the 512 px still is the fallback.
/// Everything else uses the thumbnail block (smaller), or the file when there is none.
pub(super) fn image_source(f: &Value, name: &str) -> (String, String, Option<String>) {
    let img = &f["image"];
    let still = (st(img, "url"), st(img, "scr"));
    let gif = st(f, "mimeType") == "image/gif" || st(img, "mimeType") == "image/gif" || name.to_lowercase().ends_with(".gif");
    let (fu, fs) = (st(f, "url"), st(f, "scr"));
    if gif && !fs.is_empty() && !fu.is_empty() {
        let fb = (!still.1.is_empty() && still.1 != fs).then(|| still.1.clone());
        return (fu, fs, fb);
    }
    if !still.0.is_empty() && !still.1.is_empty() { (still.0, still.1, None) } else { (fu, fs, None) }
}

/// Unique, stable cache key of one attached image: `<activity id>#<index>:<hash of url + scr>`.
pub(super) fn attachment_key(activity: &str, index: usize, url: &str, scr: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (url, scr).hash(&mut h);
    format!("{activity}#{index}:{:08x}", h.finish() as u32)
}

/// The short message (English source, translated where shown) for a rejected activity (4xx), by what was being sent.
fn friendly_failure(body: &Value) -> &'static str {
    match (body["verb"].as_str().unwrap_or(""), body["object"]["objectType"].as_str().unwrap_or("")) {
        ("add", "reaction2") => "Couldn't send the reaction",
        ("delete", _) if body["object"]["objectType"] == "activity" => "Couldn't delete the message or reaction",
        ("post", _) => "Couldn't send the message",
        ("tag" | "untag", _) => "Couldn't change the favorite",
        ("hide", _) => "Couldn't hide the space",
        ("update", _) => "Couldn't rename the space",
        ("add", _) => "Couldn't add the person",
        ("leave", _) => "Couldn't leave the space",
        _ => "Couldn't complete the action",
    }
}

#[cfg(test)]
mod tests;

mod send;
mod fold;
mod history;
mod echo;
