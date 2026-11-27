//! Selecting several messages and copying them:
//! - A press on a row with modifier Shift / Ctrl (Alt is ignored): no modifier = only that row is selected, it becomes the
//!   anchor, multi-selection ends; Ctrl = the row toggles and becomes the anchor; Shift = the rows from the anchor (the row
//!   itself when there is none) to it are selected, all others not; Ctrl+Shift does nothing. Multi-selection is on while
//!   more than one row is selected (Ctrl) / the range spans more than one row (Shift).
//! - Dragging with the button down onto other rows (no modifier) extends the range: the Shift rule.
//! - Esc in the messages view clears: nothing selected, no anchor, multi-selection off. Switching spaces clears it too.
//! - Rows that are selected while multi-selecting get message-background `checked`.
//! - Ctrl+C in the messages view and the context-menu entry "Copy selected messages" (shown alone while multi-selecting)
//!   copy the selection, see `selected_html`; the clipboard gets plain text and HTML.
use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MultiSel {
    /// Selected rows (ids), in the order they were selected.
    pub sel: Vec<String>,
    /// The anchor of Shift / drag ranges.
    pub anchor: Option<String>,
    /// Multi-selection is on.
    pub multi: bool,
}

/// The list's selection modifier: only Shift and Ctrl count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SelMod { None, Ctrl, Shift, Both }

impl SelMod {
    pub(crate) fn from(m: egui::Modifiers) -> Self {
        match (m.shift, m.ctrl || m.mac_cmd) { (false, false) => Self::None, (false, true) => Self::Ctrl, (true, false) => Self::Shift, (true, true) => Self::Both }
    }
}

impl MultiSel {
    pub(crate) fn is_selected(&self, id: &str) -> bool { self.sel.iter().any(|s| s == id) }
    /// Selected and drawn as part of a multi-selection.
    pub(crate) fn highlighted(&self, id: &str) -> bool { self.multi && self.is_selected(id) }

    /// A press on a row with a modifier; `order` = the list's rows top to bottom.
    pub(crate) fn handle(&mut self, order: &[String], id: &str, m: SelMod) {
        if !order.iter().any(|o| o == id) { return; } // out of range: nothing
        match m {
            SelMod::None => { self.sel = vec![id.to_string()]; self.anchor = Some(id.to_string()); self.multi = false; }
            SelMod::Ctrl => {
                if self.is_selected(id) { self.sel.retain(|s| s != id); } else { self.sel.push(id.to_string()); }
                self.multi = self.sel.iter().filter(|s| order.contains(s)).count() > 1;
                self.anchor = Some(id.to_string());
            }
            SelMod::Shift => self.augment(order, id),
            SelMod::Both => {}
        }
    }

    /// The rows between the anchor and `id` (inclusive), nothing else.
    pub(crate) fn augment(&mut self, order: &[String], id: &str) {
        let Some(to) = order.iter().position(|o| o == id) else { return };
        let from = self.anchor.as_ref().and_then(|a| order.iter().position(|o| o == a)).unwrap_or(to);
        if self.anchor.as_ref().is_none_or(|a| !order.contains(a)) { self.anchor = Some(id.to_string()); }
        let (a, b) = (from.min(to), from.max(to));
        self.sel = order[a..=b].to_vec();
        self.multi = b - a + 1 > 1;
    }

    /// Clears the selection.
    pub(crate) fn clear(&mut self) { *self = Self::default(); }
}

/// One list row as the copy reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CopyRow {
    pub id: String,
    /// The sender shown in the row's header; empty for grouped rows and info rows.
    pub name: String,
    /// The row's time stamp (see `copy_stamp`).
    pub stamp: String,
    /// The row is grouped under the previous one.
    pub grouped: bool,
    /// The message content as HTML (shared with the row's cached layout: recording a row each frame copies no text).
    pub body: std::sync::Arc<str>,
}

/// The selection as HTML: rows oldest first; per selected row `<p>`, then `"%1 %2 %3 "` with name, time stamp and `&bull;`
/// when the row has a name, then `"%1 %2 "` with time stamp and `&bull;` when it is grouped, has a time and the row before it
/// is not selected, then the content and `</p>`. A result of just `<p></p>` is cleared. Names and times are inserted as they
/// are (no escaping).
pub(crate) fn selected_html(rows: &[CopyRow], sel: &MultiSel) -> String {
    let mut out = String::new();
    let mut prev_selected = false;
    for r in rows {
        if !sel.is_selected(&r.id) { prev_selected = false; continue; }
        out.push_str("<p>");
        if !r.name.is_empty() { out.push_str(&format!("{} {} &bull; ", r.name, r.stamp)); }
        if !r.stamp.is_empty() && r.grouped && !prev_selected { out.push_str(&format!("{} &bull; ", r.stamp)); }
        out.push_str(&r.body);
        out.push_str("</p>");
        prev_selected = true;
    }
    if out == "<p></p>" { out.clear(); }
    out
}

/// The time stamp used when copying: "dd.MM.yyyy, HH:mm" from the row's local date and time.
pub(crate) fn copy_stamp(date: &str, time: &str) -> String {
    let d: Vec<&str> = date.split('-').collect();
    match d.as_slice() { [y, m, dd] => format!("{dd}.{m}.{y}, {time}"), _ => time.to_string() }
}

/// A message body as HTML: the rich text when there is one, else the plain text escaped, line breaks as `<br>`.
pub(crate) fn body_html(m: &Message) -> String {
    if !m.html.trim().is_empty() { return m.html.clone(); }
    let esc = m.text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    esc.replace("\r\n", "\n").replace('\n', "<br>")
}

const BLOCKS: [&str; 14] = ["p", "div", "li", "ul", "ol", "blockquote", "pre", "h1", "h2", "h3", "h4", "h5", "h6", "tr"];

/// The plain text for this HTML (approximation for the markup used here): one line per block (`<p>`, `<div>`, list items,
/// quotes, headings; a block inside an open `<p>` starts a new one), `<br>` = line break, whitespace collapsed outside `<pre>`,
/// entities decoded, tags dropped (no list markers).
pub(crate) fn html_to_plain(html: &str) -> String {
    let mut blocks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut open = false;
    let mut pre = 0usize;
    let flush = |cur: &mut String, blocks: &mut Vec<String>, open: &mut bool, force: bool| {
        if force || *open || !cur.is_empty() { blocks.push(std::mem::take(cur)); }
        *open = false;
    };
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(tag_start) = rest.strip_prefix('<') {
            let end = tag_start.find('>').unwrap_or(tag_start.len());
            let tag = &tag_start[..end];
            rest = tag_start.get(end + 1..).unwrap_or("");
            let closing = tag.starts_with('/');
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
            if name == "br" { cur.push('\n'); continue; }
            if name == "pre" { if closing { pre = pre.saturating_sub(1); } else { pre += 1; } }
            if BLOCKS.contains(&name.as_str()) {
                if closing { flush(&mut cur, &mut blocks, &mut open, false); }
                else { if !cur.is_empty() { flush(&mut cur, &mut blocks, &mut open, true); } open = true; } // an empty open block is reused
            }
            continue;
        }
        let next = rest.find('<').unwrap_or(rest.len());
        let text = decode_entities(&rest[..next]);
        rest = &rest[next..];
        if pre > 0 { cur.push_str(&text); continue; }
        for ch in text.chars() {
            if ch.is_whitespace() && ch != '\u{a0}' {
                if !cur.is_empty() && !cur.ends_with([' ', '\n']) { cur.push(' '); }
            } else { cur.push(if ch == '\u{a0}' { ' ' } else { ch }); }
        }
    }
    if !cur.is_empty() { blocks.push(cur); }
    blocks.join("\n")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.bytes().take(11).position(|b| b == b';'); // within 10 bytes: no rescan of the rest per `&`
        let ent = end.map(|e| &tail[1..e]);
        let ch = ent.and_then(|e| match e {
            "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'), "quot" => Some('"'), "apos" => Some('\''), "nbsp" => Some('\u{a0}'), "bull" => Some('•'),
            _ => e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")).and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| e.strip_prefix('#').and_then(|d| d.parse().ok())).and_then(char::from_u32),
        });
        match (ch, end) { (Some(c), Some(e)) => { out.push(c); rest = &tail[e + 1..]; } _ => { out.push('&'); rest = &tail[1..]; } }
    }
    out.push_str(rest);
    out
}

/// The selection of a space (kept in egui's temp memory, cleared when another space opens).
pub(crate) fn msel(ctx: &egui::Context, space: &str) -> MultiSel { ctx.data(|d| d.get_temp::<MultiSel>(Id::new(("msg-multisel", space)))).unwrap_or_default() }
pub(crate) fn msel_put(ctx: &egui::Context, space: &str, s: MultiSel) { ctx.data_mut(|d| d.insert_temp(Id::new(("msg-multisel", space)), s)); }

/// This frame's list rows for copying (filled while the list is drawn, in list order).
pub(super) fn record_copy(ctx: &egui::Context, r: CopyRow) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<CopyRow>>(Id::new("msg-copyrows")).push(r));
}

/// A row reports where the pointer is: pressed on it this frame (`press`), or over it.
pub(super) fn record_pointer_row(ctx: &egui::Context, id: &str, press: bool) {
    ctx.data_mut(|d| {
        d.insert_temp(Id::new("msg-pointer-row"), id.to_string());
        if press { d.insert_temp(Id::new("msg-press-row"), id.to_string()); }
    });
}

/// "Copy selected messages" of `space` with this frame's rows; false when nothing was copied.
pub(super) fn copy_selected(ctx: &egui::Context, space: &str) -> bool {
    let rows = ctx.data(|d| d.get_temp::<Vec<CopyRow>>(Id::new("msg-copyrows"))).unwrap_or_default();
    let html = selected_html(&rows, &msel(ctx, space));
    if html.is_empty() { return false; }
    to_clipboard(ctx, &html);
    true
}

impl App {
    /// After the rows were drawn: a press on a row (selection with the modifier), dragging onto other rows without a modifier
    /// (range), Esc (clear) and Ctrl+C (copy).
    pub(super) fn multi_select_input(&mut self, ctx: &egui::Context, ui: &egui::Ui, space: &str, list: Rect) {
        // Only the ids, read in place (`get_temp` would clone every row first).
        let order: Vec<String> = ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<CopyRow>>(Id::new("msg-copyrows")).iter().map(|r| r.id.clone()).collect());
        let mut s = msel(ctx, space);
        let before = s.clone();
        let mods = SelMod::from(ctx.input(|i| i.modifiers));
        let k_drag = Id::new(("msg-drag-row", space));
        let body = Rect::from_min_max(list.min, pos2(list.right() - SB_W, list.bottom())); // not the scrollbar
        let pressed = ctx.input(|i| i.pointer.primary_pressed());
        if let Some(row) = ctx.data(|d| d.get_temp::<String>(Id::new("msg-press-row"))) {
            s.handle(&order, &row, mods);
            ctx.data_mut(|d| d.insert_temp(k_drag, row));
        } else if pressed && mods == SelMod::None && super::super::widgets::hover_ok(ui, body) {
            // A press between rows (date line, start marker, empty space): that row would be selected alone - nothing visible.
            s.clear();
        }
        if ctx.input(|i| i.pointer.primary_down()) && mods == SelMod::None && ctx.data(|d| d.get_temp::<String>(k_drag)).is_some() {
            if let Some(over) = ctx.data(|d| d.get_temp::<String>(Id::new("msg-pointer-row"))) {
                if s.anchor.is_some() && (s.multi || s.anchor.as_deref() != Some(over.as_str())) { s.augment(&order, &over); }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) { ctx.data_mut(|d| d.remove::<String>(k_drag)); }
        let free = ctx.memory(|m| m.focused().is_none()); // the messages view has the keyboard, not the composer / a field
        if free && !s.sel.is_empty() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if s.multi { ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)); }
            s.clear();
        }
        if s != before { msel_put(ctx, space, s.clone()); ctx.request_repaint(); }
        // Ctrl+C (egui: Event::Copy) copies the selected messages unless text inside a message is selected (that text field
        // handles Ctrl+C itself).
        let text_sel = ctx.data(|d| d.get_temp::<(String, usize, usize)>(Id::new("msg-sel"))).is_some_and(|t| t.1 != t.2);
        if free && !text_sel && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) { copy_selected(ctx, space); }
    }
}

/// Puts the copied messages on the clipboard: plain text and HTML.
pub(crate) fn to_clipboard(ctx: &egui::Context, html: &str) {
    let plain = html_to_plain(html);
    #[cfg(windows)]
    if let Ok(mut cb) = arboard::Clipboard::new() { if cb.set().html(html, Some(&plain)).is_ok() { return; } }
    ctx.copy_text(plain); // other systems: plain text (the clipboard owner would have to stay alive for HTML)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn click_ctrl_shift_like_on_handle_selection() {
        let order = ids(&["a", "b", "c", "d", "e"]);
        let mut s = MultiSel::default();
        s.handle(&order, "b", SelMod::None);
        assert_eq!((s.sel.clone(), s.anchor.clone(), s.multi), (ids(&["b"]), Some("b".into()), false));
        s.handle(&order, "d", SelMod::Ctrl); // toggles d, d becomes the anchor, two selected -> multi
        assert!(s.is_selected("b") && s.is_selected("d") && s.multi);
        assert_eq!(s.anchor.as_deref(), Some("d"));
        s.handle(&order, "d", SelMod::Ctrl); // toggles d off: one left -> not multi
        assert!(!s.is_selected("d") && !s.multi);
        s.handle(&order, "a", SelMod::Shift); // range from the anchor (d) up to a; everything else off
        assert_eq!(s.sel, ids(&["a", "b", "c", "d"]));
        assert!(s.multi);
        assert_eq!(s.anchor.as_deref(), Some("d")); // Shift keeps the anchor
        s.handle(&order, "e", SelMod::Shift);
        assert_eq!(s.sel, ids(&["d", "e"]));
        s.handle(&order, "c", SelMod::Both); // Ctrl+Shift: nothing
        assert_eq!(s.sel, ids(&["d", "e"]));
        s.handle(&order, "c", SelMod::None); // a plain click ends the multi-selection
        assert_eq!((s.sel.clone(), s.multi), (ids(&["c"]), false));
        s.clear();
        assert_eq!(s, MultiSel::default());
    }

    #[test]
    fn shift_without_anchor_and_drag() {
        let order = ids(&["a", "b", "c"]);
        let mut s = MultiSel::default();
        s.handle(&order, "b", SelMod::Shift); // no anchor yet: the row itself
        assert_eq!((s.sel.clone(), s.multi, s.anchor.as_deref()), (ids(&["b"]), false, Some("b")));
        let mut d = MultiSel::default();
        d.handle(&order, "a", SelMod::None); // press on a
        d.augment(&order, "c"); // dragged onto c
        assert_eq!((d.sel.clone(), d.multi), (ids(&["a", "b", "c"]), true));
    }

    fn row(id: &str, name: &str, grouped: bool, body: &str) -> CopyRow { CopyRow { id: id.into(), name: name.into(), stamp: "17.11.2026, 09:41".into(), grouped, body: body.into() } }

    #[test]
    fn copy_format_of_get_selected_messages_text() {
        let rows = vec![row("1", "Mara Lind", false, "Hallo"), row("2", "", true, "zweite"), row("3", "", true, "dritte"), row("4", "Sie", false, "Antwort")];
        let mut s = MultiSel::default();
        let order = ids(&["1", "2", "3", "4"]);
        s.handle(&order, "1", SelMod::None);
        s.handle(&order, "2", SelMod::Shift);
        s.handle(&order, "4", SelMod::Ctrl);
        // 2 follows a selected row: no time; 4 has its own header.
        assert_eq!(selected_html(&rows, &s), "<p>Mara Lind 17.11.2026, 09:41 &bull; Hallo</p><p>zweite</p><p>Sie 17.11.2026, 09:41 &bull; Antwort</p>");
        assert_eq!(html_to_plain(&selected_html(&rows, &s)), "Mara Lind 17.11.2026, 09:41 • Hallo\nzweite\nSie 17.11.2026, 09:41 • Antwort");
        // A grouped row whose predecessor is not selected gets the time.
        let mut t = MultiSel::default();
        t.handle(&order, "3", SelMod::None);
        assert_eq!(selected_html(&rows, &t), "<p>17.11.2026, 09:41 &bull; dritte</p>");
        // Nothing selected / an empty row.
        assert_eq!(selected_html(&rows, &MultiSel::default()), "");
        let empty = vec![CopyRow { id: "x".into(), ..Default::default() }];
        let mut e = MultiSel::default();
        e.handle(&ids(&["x"]), "x", SelMod::None);
        assert_eq!(selected_html(&empty, &e), "");
    }

    #[test]
    fn plain_text_of_rich_messages() {
        assert_eq!(html_to_plain("<p>A &bull; <p>Zeile 1</p><p>Zeile&nbsp;2<br>3</p></p>"), "A • \nZeile 1\nZeile 2\n3");
        assert_eq!(html_to_plain("<p><blockquote>zitiert</blockquote>Antwort &amp; mehr</p>"), "zitiert\nAntwort & mehr");
        assert_eq!(html_to_plain("<p>x <ul><li>eins</li><li>zwei</li></ul></p>"), "x \neins\nzwei");
        assert_eq!(copy_stamp("2026-11-17", "09:41"), "17.11.2026, 09:41");
        let m = Message { text: "a<b\nc".into(), ..Default::default() };
        assert_eq!(body_html(&m), "a&lt;b<br>c");
    }
}

/// The composer's markdown for this HTML (the reverse of cloud/richtext.rs), for editing a message without losing its
/// formatting: paragraphs and `<br>` as lines, `<ol>` items as `1. `, `<ul>` items as `- `, `<strong>`/`<b>` as `**`, `<em>`/`<i>`
/// as `_`, `<s>` as `~~`, `<code>` as backticks, `<pre>` as a fenced block, `<blockquote>` lines with `> `, headings with `#`,
/// `<hr>` as `---`, links as `[text](url)` (just the url when the text is the url). Mentions keep their name. Other tags are dropped.
pub(crate) fn html_to_markdown(html: &str) -> String {
    let mut out = String::new();
    let mut lists: Vec<(bool, usize)> = Vec::new(); // (ordered, next number)
    let (mut quote, mut pre) = (0usize, 0usize);
    let mut link: Option<(String, usize)> = None; // (href, where its text starts in `out`)
    let line_start = |out: &mut String, quote: usize| {
        if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
        for _ in 0..quote { out.push_str("> "); }
    };
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(t) = rest.strip_prefix('<') {
            let end = t.find('>').unwrap_or(t.len());
            let tag = &t[..end];
            rest = t.get(end + 1..).unwrap_or("");
            let closing = tag.starts_with('/');
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
            let attr = |a: &str| tag.find(&format!("{a}=\"")).map(|i| { let v = &tag[i + a.len() + 2..]; v[..v.find('"').unwrap_or(v.len())].to_string() });
            match (name.as_str(), closing) {
                ("br", _) => line_start(&mut out, quote),
                ("p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                    line_start(&mut out, quote);
                    if let Some(n) = name.strip_prefix('h').and_then(|n| n.parse::<usize>().ok()) { out.push_str(&"#".repeat(n)); out.push(' '); }
                }
                ("p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {}
                ("ul", false) => lists.push((false, 1)),
                ("ol", false) => lists.push((true, attr("start").and_then(|s| s.parse().ok()).unwrap_or(1))),
                ("ul" | "ol", true) => { lists.pop(); }
                ("li", false) => {
                    line_start(&mut out, quote);
                    let depth = lists.len().saturating_sub(1);
                    out.push_str(&"   ".repeat(depth));
                    match lists.last_mut() { Some((true, n)) => { out.push_str(&format!("{n}. ")); *n += 1; } _ => out.push_str("- ") }
                }
                ("blockquote", false) => { quote += 1; line_start(&mut out, quote); }
                ("blockquote", true) => { quote = quote.saturating_sub(1); }
                ("pre", false) => { pre += 1; line_start(&mut out, quote); out.push_str("```\n"); }
                ("pre", true) => { pre = pre.saturating_sub(1); if !out.ends_with('\n') { out.push('\n'); } out.push_str("```"); }
                ("code", _) if pre == 0 => out.push('`'),
                ("strong" | "b", _) => out.push_str("**"),
                ("em" | "i", _) => out.push('_'),
                ("s" | "del" | "strike", _) => out.push_str("~~"),
                ("hr", _) => { line_start(&mut out, quote); out.push_str("---"); }
                ("a", false) => link = attr("href").map(|h| (h, out.len())),
                ("a", true) => if let Some((h, at)) = link.take() {
                    let text = out[at..].to_string();
                    if text != h && !h.is_empty() { out.truncate(at); out.push_str(&format!("[{text}]({h})")); }
                },
                _ => {}
            }
            continue;
        }
        let next = rest.find('<').unwrap_or(rest.len());
        let text = decode_entities(&rest[..next]);
        rest = &rest[next..];
        if pre > 0 { out.push_str(&text); continue; }
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' || ch == '\t' { if !out.ends_with([' ', '\n']) && !out.is_empty() { out.push(' '); } }
            else { out.push(if ch == '\u{a0}' { ' ' } else { ch }); }
        }
    }
    out.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
}

#[cfg(test)]
mod markdown_tests {
    use super::html_to_markdown as md;
    #[test]
    fn lists_and_lines_survive_editing() {
        let h = "<ol><li>DocIntel is running</li></ol><ul><li>URL: <a href=\"http://h/health\">http://h/health</a></li><li>Interval: 60 s</li></ul><p>das hät <strong>dr</strong> claude gmeint</p>";
        assert_eq!(md(h), "1. DocIntel is running\n- URL: http://h/health\n- Interval: 60 s\ndas hät **dr** claude gmeint");
    }
    #[test]
    fn links_quotes_code_and_breaks() {
        assert_eq!(md("<p>a<br>b <a href=\"https://x.y\">site</a></p>"), "a\nb [site](https://x.y)");
        assert_eq!(md("<blockquote>zitiert</blockquote><p>x</p>"), "> zitiert\nx");
        assert_eq!(md("<pre><code>let a = 1;\n</code></pre>"), "```\nlet a = 1;\n```");
        assert_eq!(md("<ol start=\"3\"><li>c</li><li>d</li></ol>"), "3. c\n4. d");
    }
}
