//! Message actions of the "···" / right-click menu that talk to the cloud: flag / unflag, the list of my flags,
//! "Als ungelesen markieren" and a message with a quote ("Nachricht zitieren").
//!
//! Protocol:
//!   * Flags: service `userApps`, create = POST `/flags` with `{"conversation-url": <activity.target.url>,
//!     "flag-item": <activity.url>, "state": "flagged"}`, list = GET `/flags?state=flagged` -> `body.items`,
//!     unflag = PUT `<flag.url>` `{"state": "unflagged"}`.
//!   * Mark unread: the activity verb "unacknowledge", with the same body as a read receipt
//!     (`{"verb": ..., "object": {"id", "objectType": "activity"}}`).
//!   * Quote: `<blockquote class='messageQuote'>{{0}}</blockquote>` with an author line inside (a person spark-mention,
//!     then `<br/>`, then the quoted text), which the reader treats as a reply quote (see ui/messages/html/
//!     `has_author_line`).
use super::*;

/// `object.displayName`/`content` pieces are HTML-escaped like richtext.rs does.
fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// Body of "flag this message".
pub(crate) fn flag_body(conversation_url: &str, activity_url: &str) -> Value {
    json!({"conversation-url": conversation_url, "flag-item": activity_url, "state": "flagged"})
}

/// Body of "unflag" (PUT on the flag's own url).
pub(crate) fn unflag_body() -> Value { json!({"state": "unflagged"}) }

/// Last path segment of a url ("…/activities/<id>" -> "<id>").
fn tail(u: &str) -> String { u.trim_end_matches('/').rsplit('/').next().unwrap_or("").split(['?', '#']).next().unwrap_or("").to_string() }

/// `GET /flags?state=flagged` -> (space id, message id, flag url) for every flagged item.
pub(crate) fn parse_flags(v: &Value) -> Vec<(String, String, String)> {
    let items = v["items"].as_array().cloned().or_else(|| v.as_array().cloned()).unwrap_or_default();
    items.iter().filter(|f| f["state"].as_str().is_none_or(|s| s == "flagged"))
        .filter_map(|f| {
            let (item, conv, url) = (f["flag-item"].as_str()?, f["conversation-url"].as_str().unwrap_or(""), f["url"].as_str().unwrap_or(""));
            let msg = tail(item);
            // The conversation url isn't always set: the activity url doesn't carry the space, so it stays empty then.
            let space = if conv.is_empty() { String::new() } else { tail(conv) };
            (!msg.is_empty()).then(|| (space, msg, url.to_string()))
        }).collect()
}

/// The activity of "Als ungelesen markieren".
pub(crate) fn unread_body(msg: &str) -> Value { json!({"verb": "unacknowledge", "object": {"id": msg, "objectType": "activity"}}) }

/// What a quoted message shared, as the official client words it in the quote (MessageStringFormatter, sent in English
/// whatever the UI language, so every client reads the same text): one file "Shared a file: {{0}}", a snapshot
/// "Shared a snapshot: {{0}}", several files "Shared {{0}} files", recordings / meeting recaps "Shared a recording: {{0}}" /
/// "Shared meeting recap: {{0}}". There's no thumbnail and no activity id in the quote: the reader finds the original by
/// author and file name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shared { File(String), Snapshot(String), Files(u32), Recording(String), Recap(String) }

impl Shared {
    /// The summary line as it is sent.
    pub fn line(&self) -> String {
        match self {
            Shared::File(n) => format!("Shared a file: {n}"),
            Shared::Snapshot(n) => format!("Shared a snapshot: {n}"),
            Shared::Files(k) => format!("Shared {k} files"),
            Shared::Recording(n) => format!("Shared a recording: {n}"),
            Shared::Recap(n) => format!("Shared meeting recap: {n}"),
        }
    }
    /// The same line in the UI's words.
    pub fn label(&self) -> String {
        match self {
            Shared::File(n) => format!("Datei geteilt: {n}"),
            Shared::Snapshot(n) => format!("Schnappschuss geteilt: {n}"),
            Shared::Files(k) => format!("{k} Dateien geteilt"),
            Shared::Recording(n) => format!("Aufzeichnung geteilt: {n}"),
            Shared::Recap(n) => format!("Besprechungszusammenfassung geteilt: {n}"),
        }
    }
    /// The file name it names (none for "Shared N files").
    pub fn name(&self) -> Option<&str> {
        match self { Shared::File(n) | Shared::Snapshot(n) | Shared::Recording(n) | Shared::Recap(n) => Some(n), Shared::Files(_) => None }
    }
    /// The summary of a message sharing these files (names in share order): none, one file, or a count.
    pub fn of(names: &[String]) -> Option<Shared> {
        match names { [] => None, [one] => Some(Shared::File(one.clone())), more => Some(Shared::Files(more.len() as u32)) }
    }
    /// A summary line (whitespace already collapsed) back into what it says; `None` for any other text.
    pub fn parse(line: &str) -> Option<Shared> {
        let l = line.trim();
        let named = |p: &str| l.strip_prefix(p).map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        if let Some(n) = named("Shared a file:") { return Some(Shared::File(n)); }
        if let Some(n) = named("Shared a snapshot:") { return Some(Shared::Snapshot(n)); }
        if let Some(n) = named("Shared a recording:") { return Some(Shared::Recording(n)); }
        if let Some(n) = named("Shared meeting recap:") { return Some(Shared::Recap(n)); }
        let k = l.strip_prefix("Shared ")?.strip_suffix(" files")?.trim().parse::<u32>().ok()?;
        (k > 1).then_some(Shared::Files(k))
    }
}

/// The file names a message shared, in order: its pictures, then its other files (the "📎 name" lines of its text).
pub fn shared_names(m: &Message) -> Vec<String> {
    let clips = m.text.lines().filter_map(|l| l.trim_start().strip_prefix('📎')).map(|n| n.trim().to_string());
    m.images.iter().map(|a| a.name.trim().to_string()).chain(clips).filter(|n| !n.is_empty()).collect()
}

/// A message's text without its "📎 name" file lines.
fn without_clips(text: &str) -> String {
    text.lines().filter(|l| !l.trim_start().starts_with('📎')).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// The HTML of a message with a quote, the official client's shape: `<blockquote class='messageQuote'>` + author (person
/// mention when the uuid is known) + `<br/>` + the summary of what the quoted message shared (`Shared`, when it shared files)
/// + `<br/>` + the quoted message's HTML (or its escaped text without the file lines) when it has text + `</blockquote>` +
/// the new text's HTML.
pub(crate) fn quote_html(author: &str, author_uuid: Option<&str>, quoted_html: &str, quoted_text: &str, shared: &[String], new_html: &str) -> String {
    let who = match author_uuid {
        Some(u) if !u.is_empty() => format!("<spark-mention data-object-type=\"person\" data-object-id=\"{}\">{}</spark-mention>", esc(u).replace('"', "&quot;"), esc(author)),
        _ => esc(author),
    };
    let text = if quoted_html.trim().is_empty() { esc(&without_clips(quoted_text)).replace('\n', "<br/>") } else { quoted_html.to_string() };
    let body = match (Shared::of(shared).map(|s| esc(&s.line())), text.trim().is_empty()) {
        (Some(s), true) => s,
        (Some(s), false) => format!("{s}<br/>{text}"),
        (None, _) => text,
    };
    format!("<blockquote class='messageQuote'>{who}<br/>{body}</blockquote>{new_html}")
}

/// The new text as HTML: what richtext renders, or (plain text without markup) one escaped paragraph with `<br/>` line breaks,
/// the same shape richtext.rs gives literal text when something in it needs markup.
pub(crate) fn text_html(r: &crate::richtext::Rendered, typed: &str) -> String {
    if !r.html.is_empty() { return r.html.clone(); }
    if typed.trim().is_empty() { return String::new(); }
    typed.split("\n\n").map(|p| format!("<p>{}</p>", esc(p).replace('\n', "<br/>"))).collect()
}

/// The same HTML for the offline sample (no uuid: the author stays plain text).
pub fn quote_preview_html(author: &str, quoted_html: &str, quoted_text: &str, shared: &[String], r: &crate::richtext::Rendered, typed: &str) -> String {
    quote_html(author, None, quoted_html, quoted_text, shared, &text_html(r, typed))
}

impl Session {
    /// A person id as the UI knows it (e-mail style or uuid) -> the uuid (same rule as activities.rs `person_uuid`).
    fn msg_uuid_of(&self, id: &str) -> Option<String> {
        if id.len() == 36 && id.matches('-').count() == 4 { return Some(id.to_string()); }
        self.uuids.get(id).or_else(|| self.uuids.get(&id.to_lowercase())).cloned().or_else(|| (id.eq_ignore_ascii_case(&self.my_email) && !self.user.is_empty()).then(|| self.user.clone()))
    }

    fn user_apps(&self, path: &str) -> Result<String, String> {
        match self.svc.get("userApps") {
            Some(u) if !u.is_empty() => Ok(format!("{u}{path}")),
            _ => Err("Markierungsstatus des Elements konnte nicht geändert werden.".into()), // QObject "Failed to change the flagged state of the item."
        }
    }

    /// GET my flags -> `Resp::Flags`. Read-only.
    pub(super) fn flags_load(&mut self) -> Result<(), String> {
        let v = self.http("GET", &self.user_apps("/flags?state=flagged")?, None)?;
        let _ = self.out.send(Resp::Flags(parse_flags(&v)));
        Ok(())
    }

    /// Flag (`flag_url` empty) or unflag (PUT on `flag_url`) a message; answers with `Resp::Flagged`.
    pub(super) fn flag(&mut self, space: &str, msg: &str, on: bool, flag_url: &str) -> Result<(), String> {
        const FAIL: &str = "Markierungsstatus des Elements konnte nicht geändert werden.";
        if on {
            let body = flag_body(&self.url("conversation", &format!("/conversations/{space}")), &self.url("conversation", &format!("/activities/{msg}")));
            let r = self.http("POST", &self.user_apps("/flags")?, Some(body)).map_err(|e| {
                dbg_conn(&format!("flag rejected: {}", e.split(" → ").nth(1).unwrap_or("").split_whitespace().next().unwrap_or(""))); FAIL.to_string()
            })?;
            let _ = self.out.send(Resp::Flagged(space.into(), msg.into(), Some(st(&r, "url"))));
        } else {
            if !flag_url.starts_with("https://") || !webex_host(flag_url) { return Err(FAIL.into()); }
            self.http("PUT", flag_url, Some(unflag_body())).map_err(|_| FAIL.to_string())?;
            let _ = self.out.send(Resp::Flagged(space.into(), msg.into(), None));
        }
        Ok(())
    }

    /// "Als ungelesen markieren" (verb unacknowledge).
    pub(super) fn mark_unread(&mut self, space: &str, msg: &str) -> Result<(), String> {
        self.submit(space, unread_body(msg)).map_err(|_| "Kann nicht als ungelesen markiert werden".to_string()) // QObject "Unable to mark as unread"
    }

    /// A message (new or thread reply) that quotes another one.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn post_quote(&mut self, space: &str, text: &str, markdown: bool, mentions: &[crate::richtext::MentionRef], quoted_html: &str, quoted_text: &str,
        shared: &[String], author: &str, author_email: &str, parent: Option<&str>) -> Result<(), String> {
        let resolved: Vec<crate::richtext::MentionRef> = mentions.iter().filter_map(|m| if m.all { Some(m.clone()) } else { self.msg_uuid_of(&m.id).map(|u| crate::richtext::MentionRef { id: u, ..m.clone() }) }).collect();
        let r = crate::richtext::render(text, markdown, &resolved);
        let uuid = self.msg_uuid_of(author_email);
        let html = quote_html(author, uuid.as_deref(), quoted_html, quoted_text, shared, &text_html(&r, text));
        // displayName: the typed text; the quoted text (or what it shared) when nothing was typed.
        let quoted_plain = Some(without_clips(quoted_text)).filter(|t| !t.is_empty()).or_else(|| Shared::of(shared).map(|s| s.line())).unwrap_or_default();
        let plain = if r.plain.trim().is_empty() { if text.trim().is_empty() { quoted_plain } else { text.to_string() } } else { r.plain.clone() };
        let rr = crate::richtext::Rendered { html, plain, mentions: r.mentions.clone() };
        let key_url = self.key_url(space)?;
        let key = self.key(&key_url)?;
        let object = activities::comment_object(&rr, text, &mut |t| jose::encrypt_dir(&key, &key_url, t.as_bytes()))?;
        let mut body = json!({"verb": "post", "encryptionKeyUrl": key_url, "object": object});
        if let Some(p) = parent { body["parent"] = json!({"id": p, "type": "reply"}); }
        self.submit(space, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_bodies_follow_the_sdk() {
        let b = flag_body("https://conv-a.wbx2.com/conversation/api/v1/conversations/c1", "https://conv-a.wbx2.com/conversation/api/v1/activities/a1");
        assert_eq!(b, json!({"conversation-url": "https://conv-a.wbx2.com/conversation/api/v1/conversations/c1", "flag-item": "https://conv-a.wbx2.com/conversation/api/v1/activities/a1", "state": "flagged"}));
        assert_eq!(unflag_body(), json!({"state": "unflagged"}));
    }

    #[test]
    fn flag_list_is_parsed() {
        let v = json!({"items": [
            {"url": "https://ua/flags/f1", "flag-item": "https://conv/activities/a1", "conversation-url": "https://conv/conversations/c1", "state": "flagged"},
            {"url": "https://ua/flags/f2", "flag-item": "https://conv/activities/a2", "state": "unflagged"},
            {"url": "https://ua/flags/f3", "flag-item": "https://conv/activities/a3/"}]});
        assert_eq!(parse_flags(&v), vec![("c1".into(), "a1".into(), "https://ua/flags/f1".into()), (String::new(), "a3".into(), "https://ua/flags/f3".into())]);
    }

    #[test]
    fn unread_and_quote_bodies() {
        assert_eq!(unread_body("a1"), json!({"verb": "unacknowledge", "object": {"id": "a1", "objectType": "activity"}}));
        let h = quote_html("Theo <Sommer>", Some("u-1"), "", "eins\nzwei & drei", &[], "<p>ok</p>");
        assert_eq!(h, "<blockquote class='messageQuote'><spark-mention data-object-type=\"person\" data-object-id=\"u-1\">Theo &lt;Sommer&gt;</spark-mention><br/>eins<br/>zwei &amp; drei</blockquote><p>ok</p>");
        assert_eq!(quote_html("A", None, "<b>x</b>", "x", &[], ""), "<blockquote class='messageQuote'>A<br/><b>x</b></blockquote>");
        let r = crate::richtext::render("hallo\nwelt", false, &[]);
        assert_eq!(text_html(&r, "hallo\nwelt"), "<p>hallo<br/>welt</p>");
        assert_eq!(text_html(&r, "  "), "");
    }

    fn msg(text: &str, images: &[&str]) -> Message {
        Message { text: text.into(), images: images.iter().map(|n| Attachment { name: n.to_string(), ..Default::default() }).collect(), ..Default::default() }
    }

    #[test]
    fn quote_of_an_image_names_the_file() {
        let m = msg("", &["IMG_0042.png"]);
        let names = shared_names(&m);
        assert_eq!(names, ["IMG_0042.png"]);
        assert_eq!(quote_html("Lina", None, &m.html, &m.text, &names, "<p>schön</p>"),
            "<blockquote class='messageQuote'>Lina<br/>Shared a file: IMG_0042.png</blockquote><p>schön</p>");
    }

    #[test]
    fn quote_of_a_file_drops_the_clip_line() {
        let m = msg("📎 Bericht <Q3>.pdf", &[]);
        let names = shared_names(&m);
        assert_eq!(quote_html("Lina", None, "", &m.text, &names, ""),
            "<blockquote class='messageQuote'>Lina<br/>Shared a file: Bericht &lt;Q3&gt;.pdf</blockquote>");
    }

    #[test]
    fn quote_of_several_files_counts_them() {
        let m = msg("📎 a.pdf\n📎 b.xlsx", &["c.png"]);
        let names = shared_names(&m);
        assert_eq!(names, ["c.png", "a.pdf", "b.xlsx"]);
        assert_eq!(quote_html("Lina", None, "", &m.text, &names, ""), "<blockquote class='messageQuote'>Lina<br/>Shared 3 files</blockquote>");
    }

    #[test]
    fn quote_of_text_with_a_file_has_summary_then_text() {
        let m = msg("Hier der Bericht\n📎 a.pdf", &[]);
        let names = shared_names(&m);
        assert_eq!(quote_html("Lina", None, "", &m.text, &names, ""), "<blockquote class='messageQuote'>Lina<br/>Shared a file: a.pdf<br/>Hier der Bericht</blockquote>");
        // The rich text (object.content) never carries the file lines.
        assert_eq!(quote_html("Lina", None, "<b>Bericht</b>", &m.text, &names, ""), "<blockquote class='messageQuote'>Lina<br/>Shared a file: a.pdf<br/><b>Bericht</b></blockquote>");
    }

    #[test]
    fn summary_lines_parse_back() {
        for s in [Shared::File("a b.png".into()), Shared::Snapshot("s.png".into()), Shared::Files(4), Shared::Recording("r.mp4".into()), Shared::Recap("Weekly".into())] {
            assert_eq!(Shared::parse(&s.line()), Some(s.clone()));
        }
        assert_eq!(Shared::parse("  Shared a file:  x.png "), Some(Shared::File("x.png".into())));
        for no in ["Shared a file:", "Shared files", "Shared 1 files", "Shared abc files", "Datei geteilt: x", "shared a file: x"] { assert_eq!(Shared::parse(no), None, "{no}"); }
        assert_eq!(Shared::File("x.png".into()).label(), "Datei geteilt: x.png");
        assert_eq!(Shared::Files(2).label(), "2 Dateien geteilt");
    }
}
