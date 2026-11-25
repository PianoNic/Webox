//! Message text: URL spans, the layout job with emoji runs, and text selection.
use super::*;

/// Byte ranges of the web links (http://, https://, www.) in a plain text: a link ends at whitespace (also a line break); trailing
/// punctuation and unmatched closing brackets stay outside it.
pub(crate) fn url_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let is_start = i == 0 || text[..i].ends_with(|ch: char| ch.is_whitespace() || "([<\"'«".contains(ch));
        if is_start && (rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.")) {
            let mut end = rest.find(char::is_whitespace).map_or(text.len(), |e| i + e);
            // Trailing punctuation belongs to the sentence, not the link; a closing bracket only when it isn't matched inside.
            // The brackets are counted once (n = counts of "()[]{}"), then kept current while the end moves left: linear.
            let mut n = [0usize; 6];
            for ch in text[i..end].chars() { if let Some(k) = "()[]{}".find(ch) { n[k] += 1; } }
            while end > i {
                let last = text[i..end].chars().next_back().unwrap_or(' ');
                let close = match last { ')' => Some(1), ']' => Some(3), '}' => Some(5), _ => None }; // its index in n (the opener is k - 1)
                if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '»' | '…') || close.is_some_and(|k| n[k] > n[k - 1]) {
                    if let Some(k) = close { n[k] -= 1; }
                    end -= last.len_utf8();
                } else { break; }
            }
            if end > i + "www.".len() { out.push((i, end)); }
            i = end.max(i + 1);
        } else { i += rest.chars().next().map_or(1, char::len_utf8); }
    }
    out
}

/// Text with the clickable parts of a message: web links (hyperlink-accent-text) and mentions of you / everybody
/// (message-bodyText-mention-self-text). Returns the layout job and the (byte range, url) of every link.
pub(super) fn body_job(text: &str, c: &Mc, me: &str, wrap: f32, sys: bool) -> (egui::text::LayoutJob, Vec<(std::ops::Range<usize>, String)>, Vec<EmojiRun>) {
    let mut spans: Vec<(usize, usize, bool)> = url_spans(text).into_iter().map(|(a, b)| (a, b, true)).collect();
    let low = text.to_lowercase();
    let mut mentions = vec!["@all".to_string(), "@alle".to_string()];
    if !me.is_empty() { mentions.push(me.to_lowercase()); }
    for key in mentions {
        let mut from = 0;
        while let Some(p) = low[from..].find(&key) {
            let (s, e) = (from + p, from + p + key.len());
            if text.is_char_boundary(s) && text.is_char_boundary(e) && !spans.iter().any(|(a, b, _)| s < *b && e > *a) { spans.push((s, e, false)); }
            from = e;
        }
    }
    spans.sort();
    let mut b = JobBuilder { job: egui::text::LayoutJob::default(), nchars: 0, runs: Vec::new(), sys,
        sysfont: FontId::new(BODY, egui::FontFamily::Name(emoji_text::FAMILY.into())), st: Style::default() };
    b.job.wrap.max_width = wrap;
    let (mut at, mut links) = (0, Vec::new());
    for (s, e, link) in spans {
        if s < at { continue; }
        if s > at { b.plain(&text[at..s], c.body); }
        if link {
            let from = b.job.text.len();
            b.raw(&text[s..e], c.accent, true);
            links.push((from..b.job.text.len(), text[s..e].to_string())); // byte range in the laid-out text (emoji are placeholders there)
        } else { b.plain(&text[s..e], c.accent); }
        at = e;
    }
    if at < text.len() { b.plain(&text[at..], c.body); }
    if b.job.sections.is_empty() { b.raw("", c.body, false); }
    (b.job, links, b.runs)
}

impl JobBuilder {
    /// Edge of an emoji picture at the current font size (17 pt at the 16 pt body size, so 34 pt inside an <h1>).
    pub(super) fn emoji_px(&self) -> f32 { EMOJI_SIZE * self.st.size.unwrap_or(BODY) / BODY * super::super::widgets::msg_scale() } // Barrierefreiheit > Schriftgröße
    pub(super) fn raw(&mut self, s: &str, color: Color32, underline: bool) {
        let size = self.st.size.unwrap_or(BODY) * super::super::widgets::msg_scale(); // Barrierefreiheit > Schriftgröße
        let font_id = if self.st.code { FontId::monospace(size * 0.92) } else if self.st.bold { semibold(size) } else { FontId::proportional(size) };
        let ul = underline || self.st.underline;
        let fmt = egui::TextFormat { font_id, color, underline: if ul { Stroke::new(1.0_f32, color) } else { Stroke::NONE }, italics: self.st.italic,
            strikethrough: if self.st.strike { Stroke::new(1.0_f32, color) } else { Stroke::NONE }, background: self.st.bg.unwrap_or(Color32::TRANSPARENT), ..Default::default() };
        self.put(s, fmt);
    }
    pub(super) fn put(&mut self, s: &str, fmt: egui::TextFormat) { self.job.append(s, 0.0, fmt); self.nchars += s.chars().count(); }
    pub(super) fn plain(&mut self, seg: &str, color: Color32) {
        let mut at = 0;
        for (s, e) in emoji_text::clusters(seg) {
            if s > at { self.raw(&emoji_text::strip_invisible(&seg[at..s]), color, false); }
            let cluster = &seg[s..e];
            match emoji_text::resolve(cluster) {
                Some(name) => { self.runs.push(EmojiRun { char_idx: self.nchars, picture: Some(name), keycap: None, orig: cluster.to_string(), size: self.emoji_px() }); self.raw(emoji_text::PLACEHOLDER, color, false); }
                // Keycaps (1️⃣) aren't in the set and no font draws them: a rounded key with the digit.
                None if cluster.ends_with('\u{20E3}') => { self.runs.push(EmojiRun { char_idx: self.nchars, picture: None, keycap: cluster.chars().next(), orig: cluster.to_string(), size: self.emoji_px() }); self.raw(emoji_text::PLACEHOLDER, color, false); }
                // Not in the picture set: the system emoji font if it has every visible code point, else hidden (never a box).
                None => if let Some(vis) = self.sys.then(|| emoji_text::sys_text(cluster)).flatten() {
                    let f = egui::TextFormat { font_id: self.sysfont.clone(), color, ..Default::default() }; self.put(&vis, f);
                },
            }
            at = e;
        }
        if at < seg.len() { self.raw(&emoji_text::strip_invisible(&seg[at..]), color, false); }
    }
}

/// Selection highlight rectangles (relative to the galley) for the chars `a..b` of the laid-out text.
pub(super) fn sel_rects(g: &Galley, a: usize, b: usize) -> Vec<Rect> {
    let (mut out, mut off) = (Vec::new(), 0usize);
    for row in &g.rows {
        let n = row.glyphs.len();
        let (lo, hi) = (a.max(off), b.min(off + n));
        if lo < hi {
            let x0 = row.glyphs[lo - off].pos.x;
            let x1 = if hi - off < n { row.glyphs[hi - off].pos.x } else { row.rect.right() };
            out.push(Rect::from_min_max(pos2(x0, row.rect.top()), pos2(x1, row.rect.bottom())));
        }
        off += n + usize::from(row.ends_with_newline);
    }
    out
}

/// Double click: the word (letters, digits, `_`), or the run of spaces / punctuation, around char `i`.
pub(super) fn word_range(text: &str, i: usize) -> (usize, usize) {
    let cs: Vec<char> = text.chars().collect();
    if cs.is_empty() { return (0, 0); }
    let i = i.min(cs.len() - 1);
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
    let k = class(cs[i]);
    let (mut a, mut b) = (i, i + 1);
    while a > 0 && class(cs[a - 1]) == k && cs[a - 1] != '\n' { a -= 1; }
    while b < cs.len() && class(cs[b]) == k && cs[b] != '\n' { b += 1; }
    (a, b)
}

/// Triple click: the paragraph (between hard line breaks) around char `i`.
pub(super) fn paragraph_range(text: &str, i: usize) -> (usize, usize) {
    let cs: Vec<char> = text.chars().collect();
    let i = i.min(cs.len());
    let a = cs[..i].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1);
    let b = cs[i..].iter().position(|c| *c == '\n').map_or(cs.len(), |p| i + p);
    (a, b)
}

/// The plain text of the selection `a..b` (chars of the laid-out text); emoji placeholders become the emoji again.
pub(super) fn selected_text(text: &str, emojis: &[EmojiRun], links: &[(std::ops::Range<usize>, String)], a: usize, b: usize) -> String {
    let (mut out, mut skip) = (String::new(), false);
    for (i, (byte, ch)) in text.char_indices().enumerate().take(b) {
        if skip { skip = false; continue; } // the placeholder's second char
        if let Some(e) = emojis.iter().find(|e| e.char_idx == i && emoji_text::PLACEHOLDER.starts_with(ch)) {
            skip = true;
            if i >= a { out.push_str(&e.orig); }
            continue;
        }
        if i >= a { out.push(ch); }
        // A fully selected link whose label isn't its target is copied with the real target: "label (https://...)".
        let end = byte + ch.len_utf8();
        if let Some((r, u)) = links.iter().find(|(r, _)| r.end == end) {
            if text[..r.start].chars().count() >= a && text[r.clone()] != *u && u != "webox-jump:" { out.push_str(&format!(" ({u})")); }
        }
    }
    out
}

pub(super) fn link_at(g: &Galley, links: &[(std::ops::Range<usize>, String)], pos: Vec2) -> Option<String> {
    if links.is_empty() { return None; }
    let r = g.rect;
    if pos.x < r.left() || pos.x > r.right() || pos.y < r.top() || pos.y > r.bottom() { return None; }
    let ci = g.cursor_from_pos(pos).ccursor.index;
    let byte = g.job.text.char_indices().nth(ci).map_or(g.job.text.len(), |(b, _)| b);
    links.iter().find(|(r, _)| byte >= r.start && byte < r.end).map(|(_, u)| u.clone())
}

/// The hover text of a link: a web link's real target (its text may name another address than it opens); None for the jump
/// link and mentions, which open nothing outside the app.
pub(super) fn link_hint(u: &str) -> Option<&str> { (u != html::JUMP && !u.starts_with(html::MENTION)).then_some(u) }

/// Middle elision for a file name.
pub(super) fn elide_middle(ui: &egui::Ui, s: &str, font: FontId, max: f32) -> String {
    elide_middle_by(s, max, &|t: &str| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x))
}

/// Middle elision (as Qt's ElideMiddle): one character is added on each side per step while the two parts plus "…" still
/// fit, so both sides keep the same number of characters ("Spesenabre…_2026.xltx", 10 + 10).
pub(super) fn elide_middle_by(s: &str, max: f32, w: &dyn Fn(&str) -> f32) -> String {
    if w(s) <= max { return s.to_string(); }
    let chars: Vec<char> = s.chars().collect();
    let cut = |n: usize| format!("{}…{}", chars[..n].iter().collect::<String>(), chars[chars.len() - n..].iter().collect::<String>());
    let mut best = "…".to_string();
    for n in 1..chars.len().div_ceil(2) {
        let cand = cut(n);
        if w(&cand) > max { break; }
        best = cand;
    }
    best
}

/// How far a block quote's text sits right of the one around it (its bar is drawn in that gap).
pub(super) const QUOTE_INDENT: f32 = 14.0;

/// The nesting depth of the quote (a, b): 1 for an outermost one.
pub(super) fn quote_depth(quotes: &[(usize, usize)], a: usize, b: usize) -> usize { quotes.iter().filter(|q| q.0 <= a && b <= q.1).count() }
pub(super) fn quote_depth_max(quotes: &[(usize, usize)]) -> usize { quotes.iter().map(|q| quote_depth(quotes, q.0, q.1)).max().unwrap_or(0) }

/// Shifts every row that starts inside a block quote (wrapped rows too, which leading spaces could not indent) right by
/// `QUOTE_INDENT` per nesting level. Glyphs, mesh and rects move together, so cursor, selection and link hits stay right.
pub(super) fn indent_quotes(g: std::sync::Arc<Galley>, quotes: &[(usize, usize)]) -> std::sync::Arc<Galley> {
    if quotes.is_empty() { return g; }
    let mut gal = (*g).clone();
    let mut at = 0usize;
    for row in &mut gal.rows {
        let depth = quotes.iter().filter(|q| q.0 <= at && at < q.1).count();
        at += row.char_count_including_newline();
        if depth == 0 { continue }
        let dx = egui::vec2(depth as f32 * QUOTE_INDENT, 0.0);
        row.rect = row.rect.translate(dx);
        for gl in &mut row.glyphs { gl.pos += dx; }
        row.visuals.mesh.translate(dx);
        row.visuals.mesh_bounds = row.visuals.mesh_bounds.translate(dx);
        gal.rect = gal.rect.union(row.rect);
        gal.mesh_bounds = gal.mesh_bounds.union(row.visuals.mesh_bounds);
    }
    std::sync::Arc::new(gal)
}

/// The layout job of plugin-given spans (`kept::SPAN_*`; an earlier text, a diff, a dialog tab): `base` for plain text (struck
/// through for `SPAN_OLD`), `added` for added parts, `removed` struck through for removed ones. The whole line is trimmed.
pub(crate) fn spans_job(spans: &[crate::cloud::Span], font: FontId, base: Color32, added: Color32, removed: Color32, wrap: f32) -> egui::text::LayoutJob {
    use crate::extras::plugins::kept::{SPAN_ADDED, SPAN_OLD, SPAN_REMOVED};
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    let n = spans.len();
    for (i, (t, k)) in spans.iter().enumerate() {
        let t = normalize_breaks(t);
        let t = if i == 0 { t.trim_start() } else { t.as_str() };
        let t = if i + 1 == n { t.trim_end() } else { t };
        if t.is_empty() { continue; }
        let (color, strike) = match *k { SPAN_ADDED => (added, false), SPAN_REMOVED => (removed, true), SPAN_OLD => (base, true), _ => (base, false) };
        job.append(t, 0.0, egui::TextFormat { font_id: font.clone(), color, strikethrough: if strike { Stroke::new(1.0_f32, color) } else { Stroke::NONE }, ..Default::default() });
    }
    job
}

/// A plugin's diff view of the current text (`Message::current`): the added parts of `job` (a message's laid-out text) in the
/// added colour. Only when `job` shows exactly the text the spans spell (emoji placeholders or rich text that changed it leave
/// it as it is).
pub(super) fn mark_spans(job: &mut egui::text::LayoutJob, spans: &[crate::cloud::Span], c: &Mc) {
    use crate::extras::plugins::kept::SPAN_ADDED;
    if !spans.iter().any(|s| s.1 == SPAN_ADDED) { return; }
    let full: String = spans.iter().map(|s| s.0.as_str()).collect();
    let (ft, lead) = (full.trim(), full.len() - full.trim_start().len());
    if ft.is_empty() || job.text.trim() != ft { return; }
    let Some(off) = job.text.find(ft) else { return };
    // Byte ranges of the added parts in `job.text`.
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut at = 0;
    for (t, k) in spans {
        let (a, b) = (at, at + t.len());
        at = b;
        if *k != SPAN_ADDED { continue; }
        let (a, b) = (a.max(lead), b.min(lead + ft.len()));
        if a < b { ranges.push(a - lead + off..b - lead + off); }
    }
    let mut out = Vec::with_capacity(job.sections.len() + 2 * ranges.len());
    for s in std::mem::take(&mut job.sections) {
        // Cut the section at the range edges inside it; the pieces in a range take the added colour.
        let mut cuts = vec![s.byte_range.start, s.byte_range.end];
        for r in &ranges { for x in [r.start, r.end] { if x > s.byte_range.start && x < s.byte_range.end { cuts.push(x); } } }
        cuts.sort_unstable();
        cuts.dedup();
        for (j, w) in cuts.windows(2).enumerate() {
            let mut p = s.clone();
            p.byte_range = w[0]..w[1];
            if j > 0 { p.leading_space = 0.0; }
            if ranges.iter().any(|r| r.start <= w[0] && w[1] <= r.end) { p.format.color = c.added; }
            out.push(p);
        }
    }
    job.sections = out;
}

#[cfg(test)]
mod tests {
    use super::{link_hint, url_spans};
    fn urls(t: &str) -> Vec<&str> { url_spans(t).into_iter().map(|(a, b)| &t[a..b]).collect() }

    #[test]
    fn trailing_brackets_are_balanced_in_one_pass() {
        assert_eq!(urls("(siehe https://x.example/(a)(b)))."), ["https://x.example/(a)(b)"]);
        assert_eq!(urls("https://x.example/[1]], (https://y.example/{k}})"),["https://x.example/[1]", "https://y.example/{k}"]);
        // Thousands of unmatched closing brackets: all of them stay outside the link (each step used to recount the whole URL).
        let t = format!("https://x.example/{}", ")".repeat(50_000));
        assert_eq!(urls(&t), ["https://x.example/"]);
    }

    #[test]
    fn nested_reply_quote_indents_every_row_and_nests() {
        // The real message: a reply quoting a reply that itself quotes the first message, then the new text.
        let h = r#"<blockquote class="messageQuote"><span class="spark-contact">Niclas Erismann</span><br><blockquote class="messageQuote"><p>Auso, wa isch jetzt mit dine 20k Draft issues?<br><a href="https://x.example/q">https://x.example/q</a></p></blockquote><p>das sind meistens stories wo claude aglegt het. s video züg isch parkiert bis de rest stabil lauft</p></blockquote>Machsch du die sache?"#;
        let ctx = eframe::egui::Context::default();
        let _ = ctx.run(Default::default(), |_| {});
        let p = super::html::html_job(h, &super::super::mc(&crate::dark()), "", 260.0, false).unwrap();
        assert_eq!(super::quote_depth_max(&p.quotes), 2);
        let g = super::indent_quotes(ctx.fonts(|f| f.layout_job(p.job.clone())), &p.quotes);
        let row_text = |r: &eframe::egui::epaint::text::Row| r.glyphs.iter().map(|g| g.chr).collect::<String>();
        let x_of = |needle: &str| g.rows.iter().find(|r| row_text(r).contains(needle)).map(|r| r.rect.left()).expect(needle);
        assert_eq!(x_of("Niclas"), super::QUOTE_INDENT); // outer quote
        assert_eq!(x_of("Auso"), 2.0 * super::QUOTE_INDENT); // nested quote
        assert_eq!(x_of("Machsch"), 0.0); // the new text is outside
        // Every wrapped row of the outer quote's own paragraph keeps the indent (it used to fall back under the bar).
        let outer: Vec<_> = g.rows.iter().filter(|r| { let t = row_text(r); t.contains("stories") || t.contains("parkiert") || t.contains("lauft") }).collect();
        assert!(outer.len() >= 2, "paragraph should wrap at this width");
        assert!(outer.iter().all(|r| r.rect.left() == super::QUOTE_INDENT));
    }

    #[test]
    fn link_tooltip_is_the_real_target() {
        assert_eq!(link_hint("https://evil.example/login"), Some("https://evil.example/login"));
        assert_eq!(link_hint(super::html::JUMP), None);
        assert_eq!(link_hint(&format!("{}p1\u{1}Mara", super::html::MENTION)), None);
    }

    #[test]
    fn diff_spans_colour_the_history_and_the_current_text() {
        use super::*;
        use crate::extras::plugins::kept::{SPAN_ADDED, SPAN_OLD, SPAN_PLAIN, SPAN_REMOVED};
        let c = mc(&crate::dark());
        let sp = |v: &[(&str, u8)]| v.iter().map(|(t, k)| (t.to_string(), *k)).collect::<Vec<_>>();
        let j = spans_job(&sp(&[(" Hallo ", SPAN_PLAIN), ("Welt", SPAN_REMOVED), ("Leute ", SPAN_ADDED)]), FontId::proportional(12.0), c.head, c.added, c.danger, 300.0);
        assert_eq!(j.text, "Hallo WeltLeute");
        assert_eq!(j.sections.iter().map(|s| (s.format.color, s.format.strikethrough.width > 0.0)).collect::<Vec<_>>(), [(c.head, false), (c.danger, true), (c.added, false)]);
        assert!(spans_job(&sp(&[("alt", SPAN_OLD)]), FontId::proportional(12.0), c.head, c.added, c.danger, 300.0).sections[0].format.strikethrough.width > 0.0);
        // The current text: the added part of a plain message turns green, the rest keeps its colour.
        let (mut job, _, _) = body_job("Hallo liebe Leute", &c, "", 300.0, false);
        mark_spans(&mut job, &sp(&[("Hallo ", SPAN_PLAIN), ("liebe ", SPAN_ADDED), ("Leute", SPAN_PLAIN)]), &c);
        let col: Vec<(&str, Color32)> = job.sections.iter().map(|s| (&job.text[s.byte_range.clone()], s.format.color)).collect();
        assert_eq!(col, [("Hallo ", c.body), ("liebe ", c.added), ("Leute", c.body)]);
        // Spans that do not spell the shown text change nothing.
        let (mut job, _, _) = body_job("Etwas anderes", &c, "", 300.0, false);
        mark_spans(&mut job, &sp(&[("Hallo ", SPAN_PLAIN), ("neu", SPAN_ADDED)]), &c);
        assert_eq!(job.sections.len(), 1);
    }
}
