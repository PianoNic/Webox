//! The message's rich text: `object.content` (the decrypted HTML the sender's client produced) is drawn, not the plain
//! `displayName`: links keep their real target (%20 and all), `<br>` is a hard line break, mentions are `<spark-mention>`.
//! This is a small HTML-subset reader (a, br, p/div, b/strong, i/em, u, s/strike/del, code, pre, blockquote, ul/ol/li, h1-h6,
//! spark-mention, entities) that lays out into the same single `LayoutJob` the plain-text renderer uses, so emoji pictures, link
//! hit-testing, selection and copy all work unchanged:
//!   * lists become lines with "• " / "1. " in front, quotes get an em-space indent per level plus a bar drawn from `quotes`,
//!   * blocks are separated by one line break (no 12 px paragraph margins: Webex clients send <br>, not <p>),
//!   * bare URLs in text nodes outside <a> are linked too; href values are never percent-decoded (only HTML entities are),
//!   * mentions use the theme tokens message-bodyText-mention-self/all-text (accent) and -other-text (white @ 70 %).
use super::*;
use std::ops::Range;

pub(super) struct Parsed { pub job: egui::text::LayoutJob, pub links: Vec<(Range<usize>, String)>, pub runs: Vec<EmojiRun>, pub quotes: Vec<(usize, usize)>, pub reply_quote: Option<String> }

/// Link target of "Zur Originalnachricht springen" (handled by the message list, never opened).
pub(super) const JUMP: &str = "webox-jump:";
/// Link target of a person mention: `webox-mention:<person id>\u{1}<shown name>` (a click opens the contact card).
pub(super) const MENTION: &str = "webox-mention:";

enum Ev { Open(String, Vec<(String, String)>), Close(String), Text(String) }

const VOID: [&str; 7] = ["br", "hr", "img", "wbr", "meta", "input", "link"];
/// Open elements beyond this depth are ignored (see `Doc::open`).
const MAX_DEPTH: usize = 32;

fn decode_entities(s: &str) -> String {
    let (mut out, mut rest) = (String::with_capacity(s.len()), s);
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let semi = rest.bytes().take(12).position(|b| b == b';');
        let ch = semi.and_then(|e| match &rest[1..e] {
            "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'), "quot" => Some('"'), "apos" => Some('\''), "nbsp" => Some('\u{a0}'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        });
        match (ch, semi) { (Some(c), Some(e)) => { out.push(c); rest = &rest[e + 1..]; } _ => { out.push('&'); rest = &rest[1..]; } }
    }
    out + rest
}

/// For every byte position p: the first `>` outside quotes when scanning from p with no quote open (usize::MAX = none), so the
/// end of the tag that starts at `<` = i is `ends[i + 1]`. Computed once from the right: a scan from p reaches the same state as
/// one from p + 1 (or from behind the closing quote), so a message full of `<` without `>` stays linear instead of rescanning
/// to its end for every `<`.
fn tag_ends(b: &[u8]) -> Vec<usize> {
    let mut ends = vec![usize::MAX; b.len() + 1];
    let (mut dq, mut sq) = (usize::MAX, usize::MAX); // the next `"` / `'` after p
    for p in (0..b.len()).rev() {
        let after = |q: usize| if q == usize::MAX { usize::MAX } else { ends[q + 1] }; // an unclosed quote hides every later `>`
        let e = match b[p] { b'>' => p, b'"' => after(dq), b'\'' => after(sq), _ => ends[p + 1] };
        ends[p] = e;
        match b[p] { b'"' => dq = p, b'\'' => sq = p, _ => {} }
    }
    ends
}

fn parse_tag(inner: &str) -> Option<(bool, String, Vec<(String, String)>, bool)> {
    let closing = inner.starts_with('/');
    let body = inner.trim_start_matches('/');
    if !body.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) { return None; }
    let self_close = inner.ends_with('/');
    let name_end = body.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(body.len());
    let name = body[..name_end].to_lowercase();
    let mut attrs = Vec::new();
    let mut rest = &body[name_end..];
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() { break; }
        let ne = rest.find(|c: char| c == '=' || c.is_whitespace() || c == '/').unwrap_or(rest.len());
        let an = rest[..ne].to_lowercase();
        rest = rest[ne..].trim_start();
        let mut val = String::new();
        if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let e = r[1..].find(q).map_or(r.len(), |e| e + 1);
                val = decode_entities(&r[1..e]);
                rest = r.get(e + 1..).unwrap_or("");
            } else {
                let e = r.find(char::is_whitespace).unwrap_or(r.len());
                val = decode_entities(&r[..e]);
                rest = &r[e..];
            }
        }
        if !an.is_empty() { attrs.push((an, val)); }
    }
    Some((closing, name, attrs, self_close))
}

fn tokenize(h: &str) -> Vec<Ev> {
    let (mut ev, mut i, mut text_start) = (Vec::new(), 0usize, 0usize);
    let b = h.as_bytes();
    let ends = tag_ends(b);
    let flush = |ev: &mut Vec<Ev>, a: usize, z: usize| { if z > a { ev.push(Ev::Text(decode_entities(&h[a..z]))); } };
    while i < b.len() {
        if b[i] == b'<' {
            if h[i..].starts_with("<!--") {
                flush(&mut ev, text_start, i);
                i = h[i..].find("-->").map_or(h.len(), |e| i + e + 3);
                text_start = i;
                continue;
            }
            if let Some(end) = Some(ends[i + 1]).filter(|&e| e != usize::MAX) {
                let inner = &h[i + 1..end];
                if inner.starts_with('!') || inner.starts_with('?') { flush(&mut ev, text_start, i); i = end + 1; text_start = i; continue; }
                if let Some((closing, name, attrs, self_close)) = parse_tag(inner) {
                    flush(&mut ev, text_start, i);
                    if closing { ev.push(Ev::Close(name)); }
                    else {
                        let void = VOID.contains(&name.as_str());
                        ev.push(Ev::Open(name.clone(), attrs));
                        if self_close && !void { ev.push(Ev::Close(name)); }
                    }
                    i = end + 1;
                    text_start = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    flush(&mut ev, text_start, h.len());
    ev
}

/// Whether the quote that starts right before `rest` has an author line: at least two separate text pieces before its end.
fn has_author_line(rest: &[Ev]) -> bool {
    let (mut depth, mut texts) = (0usize, 0usize);
    for e in rest {
        match e {
            Ev::Open(n, _) if n == "blockquote" => depth += 1,
            Ev::Close(n) if n == "blockquote" => { if depth == 0 { break; } depth -= 1; }
            Ev::Text(t) if !t.trim().is_empty() => texts += 1,
            _ => {}
        }
    }
    texts >= 2
}

/// Body-size factor of a heading (Qt rich-text rules, see `open`): h1 2.0, h2 1.5, h3 1.2, h4 1.0, h5 0.8, h6 unchanged.
fn heading_scale(tag: &str) -> f32 {
    match tag { "h1" => 2.0, "h2" => 1.5, "h3" => 1.2, "h5" => 0.8, _ => 1.0 }
}

fn is_me(me: &str, id: &str) -> bool {
    !me.is_empty() && !id.is_empty() && (me == id || crate::jose::unb64(me).ok().and_then(|b| String::from_utf8(b).ok()).is_some_and(|s| s.ends_with(id)))
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut sp = false;
    for ch in s.chars() { if ch.is_whitespace() && ch != '\u{a0}' { if !sp { out.push(' '); } sp = true; } else { out.push(ch); sp = false; } }
    out
}

struct Doc<'a> {
    b: JobBuilder, c: &'a Mc, me: &'a str,
    stack: Vec<(String, Style, Option<Color32>)>, color: Option<Color32>,
    nls: u32, lists: Vec<(bool, u32)>, bullet: Option<String>,
    quote_depth: usize, quote_start: Vec<Option<usize>>, quotes: Vec<(usize, usize)>,
    pre: u32, link: Option<(String, Option<usize>)>, links: Vec<(Range<usize>, String)>,
    /// A reply's quote (the first <blockquote>, "Author" line + quoted text): `author_pending` until the author line is written,
    /// `reply_depth` = quote depth of that block, `reply_text` = the quoted text (to find the quoted message).
    author_pending: bool, reply_depth: usize, reply_text: String, reply_seen: bool,
    /// The <br> right after the author line only ends that line (the header already did).
    absorb_br: bool,
}

impl Doc<'_> {
    fn raw_plain(&mut self, s: &str) { let st = std::mem::take(&mut self.b.st); self.b.raw(s, self.c.body, false); self.b.st = st; }

    /// Emits the pending line breaks and the line's indent / bullet, right before visible content.
    fn flush(&mut self) {
        let started = !self.b.job.text.is_empty();
        let n = if started { self.nls.min(2) } else { 0 };
        self.nls = 0;
        for _ in 0..n { self.raw_plain("\n"); }
        if n > 0 || !started || self.b.job.text.ends_with('\n') {
            if self.quote_depth > 0 {
                let at = self.b.nchars;
                for s in self.quote_start.iter_mut() { if s.is_none() { *s = Some(at); } }
                for _ in 0..self.quote_depth { self.raw_plain("\u{2002}"); }
            }
            if let Some(bt) = self.bullet.take() {
                for _ in 0..self.lists.len() { self.raw_plain("\u{2003}"); }
                self.raw_plain(&bt);
            }
        }
    }

    fn put(&mut self, seg: &str, color: Color32, underline: bool) {
        if seg.is_empty() { return; }
        let was = self.b.st.underline;
        if underline { self.b.st.underline = true; }
        self.b.plain(seg, color);
        self.b.st.underline = was;
    }

    /// The header of a reply quote: "<Author> • Zur Originalnachricht springen" (author and bullet dim, the link in the link colour).
    fn reply_header(&mut self, author: &str) {
        self.flush();
        let c = self.c;
        let was = self.b.st.size;
        self.b.st.size = Some(SECOND);
        self.b.plain(author.trim(), c.head);
        self.b.plain(" \u{2022} ", c.head);
        let from = self.b.job.text.len();
        self.b.plain("Zur Originalnachricht springen", c.accent);
        self.links.push((from..self.b.job.text.len(), JUMP.to_string()));
        self.b.st.size = was;
        self.nls = self.nls.max(1);
        self.absorb_br = true;
    }

    fn text(&mut self, raw: &str) {
        if self.author_pending {
            let a = collapse(raw);
            if a.trim().is_empty() { return; }
            self.author_pending = false;
            self.reply_header(&a);
            return;
        }
        self.absorb_br = false;
        if self.reply_depth > 0 && self.quote_depth >= self.reply_depth { self.reply_text.push_str(raw); }
        let mut t = if self.pre > 0 { raw.trim_end_matches('\n').to_string() } else { collapse(raw) };
        if self.pre == 0 {
            let jt = &self.b.job.text;
            if t.starts_with(' ') && (self.nls > 0 || jt.is_empty() || jt.ends_with('\n') || jt.ends_with(' ') || self.bullet.is_some()) { t = t.trim_start().to_string(); }
        }
        if t.is_empty() { return; }
        let spans = if self.link.is_none() && self.pre == 0 { url_spans(&t) } else { Vec::new() };
        self.flush();
        if let Some((_, start)) = &mut self.link { if start.is_none() { *start = Some(self.b.job.text.len()); } }
        let col = if self.link.as_ref().is_some_and(|l| !l.0.starts_with(MENTION)) { self.c.accent } else { self.color.unwrap_or(self.c.body) };
        let mut at = 0;
        for (s, e) in spans {
            self.put(&t[at..s], col, false);
            let from = self.b.job.text.len();
            self.put(&t[s..e], self.c.accent, true);
            self.links.push((from..self.b.job.text.len(), t[s..e].to_string()));
            at = e;
        }
        self.put(&t[at..], col, false);
    }

    fn open(&mut self, name: &str, attrs: &[(String, String)]) {
        let attr = |k: &str| attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_str()).unwrap_or("");
        let saved = (name.to_string(), self.b.st.clone(), self.color);
        let block = |s: &mut Self| s.nls = s.nls.max(1);
        // Deeper than MAX_DEPTH open elements: only a block's line break counts (no style, indent or list level, not on the
        // stack), so nesting can't make every line and every close tag cost more and more.
        if !VOID.contains(&name) && self.stack.len() >= MAX_DEPTH {
            if matches!(name, "p" | "div" | "section" | "table" | "tr" | "ul" | "ol" | "li" | "blockquote" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") { block(self); }
            return;
        }
        match name {
            "br" => if self.absorb_br { self.absorb_br = false } else { self.nls = (self.nls + 1).min(3) },
            "hr" => block(self),
            "img" => { let alt = attr("alt"); if !alt.is_empty() { self.text(alt); } }
            "p" | "div" | "section" | "table" | "tr" => block(self),
            "ul" | "ol" => { block(self); self.lists.push((name == "ol", 0)); }
            "li" => {
                block(self);
                self.bullet = Some(match self.lists.last_mut() { Some((true, n)) => { *n += 1; format!("{n}. ") } _ => "• ".to_string() });
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => { block(self);
                // Qt rich-text heading rules: font size adjustment h1 +3, h2 +2, h3 +1, h4 0, h5 -1, none for h6, scaled by
                // {0.7, 0.8, 1.0, 1.2, 1.5, 2, 2.4}[adjustment + 2]; h1..h5 are bold, h6 is not. (The block margins have no equivalent in
                // this line-based layout.)
                self.b.st.bold = name != "h6";
                self.b.st.size = Some(BODY * heading_scale(name)); }
            "blockquote" => { block(self); self.quote_depth += 1; self.quote_start.push(None); }
            "pre" => { block(self); self.pre += 1; self.b.st.code = true; self.b.st.bg = Some(self.c.code_bg); }
            "code" => { self.b.st.code = true; self.b.st.bg = Some(self.c.code_bg); }
            "b" | "strong" => self.b.st.bold = true,
            "i" | "em" => self.b.st.italic = true,
            "u" => self.b.st.underline = true,
            "s" | "strike" | "del" => self.b.st.strike = true,
            "a" => { self.link = Some((attr("href").to_string(), None)); self.b.st.underline = true; }
            "spark-mention" => {
                let all = attr("data-object-type").eq_ignore_ascii_case("groupMention") || attr("data-group-type") == "all";
                let mine = attr("data-object-type") == "person" && is_me(self.me, attr("data-object-id"));
                self.color = Some(if all || mine { self.c.accent } else { self.c.mention_other });
                // A person mention is clickable (not inside a real link, not in a reply's author line).
                let pid = attr("data-object-id");
                if attr("data-object-type") == "person" && !pid.is_empty() && self.link.is_none() && !self.author_pending { self.link = Some((format!("{MENTION}{pid}"), None)); }
            }
            _ => {}
        }
        if !VOID.contains(&name) { self.stack.push(saved); }
    }

    fn end(&mut self, n: &str) {
        match n {
            "p" | "div" | "section" | "table" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.nls = self.nls.max(1),
            "ul" | "ol" => { self.lists.pop(); self.nls = self.nls.max(1); }
            "pre" => { self.pre = self.pre.saturating_sub(1); self.nls = self.nls.max(1); }
            "blockquote" => {
                if self.reply_depth == self.quote_depth { self.reply_depth = 0; }
                self.quote_depth = self.quote_depth.saturating_sub(1);
                if let Some(Some(s)) = self.quote_start.pop() { self.quotes.push((s, self.b.nchars)); }
                self.nls = self.nls.max(1);
            }
            "a" => {
                if let Some((href, Some(start))) = self.link.take() {
                    let end = self.b.job.text.len();
                    if end > start { self.links.push((start..end, href)); }
                }
            }
            // End of a clickable mention: the target carries the shown name.
            "spark-mention" if self.link.as_ref().is_some_and(|l| l.0.starts_with(MENTION)) => {
                if let Some((href, start)) = self.link.take() {
                    if let Some(start) = start {
                        let end = self.b.job.text.len();
                        if end > start { let name = self.b.job.text[start..end].trim().trim_start_matches('@').to_string(); self.links.push((start..end, format!("{href}\u{1}{name}"))); }
                    }
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        let Some(pos) = self.stack.iter().rposition(|e| e.0 == name) else { return };
        while self.stack.len() > pos {
            let (n, st, col) = self.stack.pop().unwrap();
            self.end(&n);
            self.b.st = st;
            self.color = col;
        }
    }
}

/// Lays out `html`; `me` is my person id (to tell my mentions from others'). `None` when there's nothing visible to show,
/// so the caller falls back to the plain text.
pub(super) fn html_job(html: &str, c: &Mc, me: &str, wrap: f32, sys: bool) -> Option<Parsed> {
    let mut b = JobBuilder { job: egui::text::LayoutJob::default(), nchars: 0, runs: Vec::new(), sys,
        sysfont: FontId::new(BODY, egui::FontFamily::Name(emoji_text::FAMILY.into())), st: Style::default() };
    b.job.wrap.max_width = wrap;
    let mut d = Doc { b, c, me, stack: Vec::new(), color: None, nls: 0, lists: Vec::new(), bullet: None, quote_depth: 0, quote_start: Vec::new(), quotes: Vec::new(), pre: 0, link: None, links: Vec::new(), author_pending: false, reply_depth: 0, reply_text: String::new(), reply_seen: false, absorb_br: false };
    let events = tokenize(html);
    for (i, ev) in events.iter().enumerate() {
        match ev {
            Ev::Open(n, a) => {
                // The first <blockquote>, at the very start and with an author line before the quoted text, is a reply's quote.
                if n == "blockquote" && !d.reply_seen && d.b.job.text.is_empty() && d.quote_depth == 0 && d.stack.len() < MAX_DEPTH && has_author_line(&events[i + 1..]) {
                    d.reply_seen = true;
                    d.author_pending = true;
                    d.open(n, a);
                    d.reply_depth = d.quote_depth;
                } else { d.open(n, a) }
            }
            Ev::Close(n) => d.close(n),
            Ev::Text(t) => d.text(t),
        }
    }
    while let Some(top) = d.stack.last().map(|e| e.0.clone()) { d.close(&top); }
    if (d.b.job.text.trim().is_empty() && d.b.runs.is_empty()) || d.b.job.sections.is_empty() { return None; } // emoji are whitespace placeholders in the text
    let reply_quote = d.reply_seen.then(|| collapse(&d.reply_text).trim().to_string()).filter(|t| !t.is_empty());
    Some(Parsed { job: d.b.job, links: d.links, runs: d.b.runs, quotes: d.quotes, reply_quote })
}

#[cfg(test)]
mod tests;
