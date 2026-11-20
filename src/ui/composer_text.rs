//! Pure text logic of the message composer (no egui): emoticon replacement, @mention tokens and the message HTML they
//! become, the `:name` / `@name` trigger detection and the markdown / formatting edits behind the toolbar and shortcuts.
//! All positions are CHAR indices (egui's cursor unit) unless a name says `byte`.
//!
//! The emoticon keys are `:)` `:(` `:D` `:O` `:S` `:/` `:|` `:'(` `;)` `XD` `-_-` (`<3` is not one); the emoji each key becomes
//! follows the usual Slack/Twitter mapping.

pub(crate) const MAX_CHARS: usize = 7439; // Webex message size limit (public Webex API documentation)

/// (emoticon, emoji), see the module doc.
pub(crate) const EMOTICONS: [(&str, &str); 11] = [
    (":)", "🙂"), (":(", "🙁"), (":D", "😃"), (":O", "😮"), (":S", "😕"), (":/", "😕"), (":|", "😐"), (":'(", "😢"), (";)", "😉"), ("XD", "😆"), ("-_-", "😑"),
];

pub(crate) fn emoticon_for(token: &str) -> Option<&'static str> {
    EMOTICONS.iter().find(|(k, _)| *k == token || (k.chars().all(|c| c.is_ascii_alphabetic() || ":;".contains(c)) && k.eq_ignore_ascii_case(token) && token.chars().any(|c| c.is_ascii_alphabetic()))).map(|(_, e)| *e)
}

pub(crate) fn byte_of(s: &str, char_idx: usize) -> usize { s.char_indices().nth(char_idx).map_or(s.len(), |(b, _)| b) }
pub(crate) fn char_of(s: &str, byte: usize) -> usize { s[..byte.min(s.len())].chars().count() }

/// "Space released" case: the word that ends right before `cursor` (the typed space is at `cursor - 1`) becomes an emoji when it is
/// exactly an emoticon. Returns the new text and the new cursor.
pub(crate) fn replace_emoticon_before_space(text: &str, cursor: usize) -> Option<(String, usize)> {
    if cursor == 0 { return None; }
    let chars: Vec<char> = text.chars().collect();
    if chars.get(cursor - 1) != Some(&' ') && chars.get(cursor - 1) != Some(&'\n') { return None; }
    replace_emoticon_word_ending_at(text, cursor - 1).map(|(t, e)| (t, e + 1))
}

/// The word that ends at `end` becomes an emoji when it is exactly an emoticon; returns the new text and the new `end`.
pub(crate) fn replace_emoticon_word_ending_at(text: &str, end: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    if end > chars.len() { return None; }
    let mut start = end;
    while start > 0 && !chars[start - 1].is_whitespace() { start -= 1; }
    let tok: String = chars[start..end].iter().collect();
    let em = emoticon_for(&tok)?;
    let mut out: String = chars[..start].iter().collect();
    out.push_str(em);
    out.extend(chars[end..].iter());
    Some((out, start + em.chars().count()))
}

/// Before sending: every whitespace-delimited word that is an emoticon becomes its emoji.
pub(crate) fn parse_emoticons(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| { if let Some(e) = emoticon_for(word) { out.push_str(e) } else { out.push_str(word) } word.clear(); };
    for c in text.chars() {
        if c.is_whitespace() { flush(&mut word, &mut out); out.push(c); } else { word.push(c); }
    }
    flush(&mut word, &mut out);
    out
}

// Mentions.

/// A person (or everybody) mentioned in the draft. `id` is the person id (the "all" mention has none).
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct Mention { pub id: String, pub name: String, pub all: bool }

impl Mention {
    /// The text the mention has in the box.
    pub(crate) fn label(&self) -> String { format!("@{}", self.name) }
}

/// `@` trigger: the `@` must start the text or follow whitespace; the query is the text up to the cursor (no line break, ≤ 40 chars).
/// Returns (char index of the `@`, query).
pub(crate) fn mention_query(text: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut i = cursor;
    while i > 0 {
        let c = chars[i - 1];
        if c == '@' {
            let ok = i == 1 || chars[i - 2].is_whitespace();
            return ok.then(|| (i - 1, chars[i..cursor].iter().collect()));
        }
        if c == '\n' || cursor - i >= 40 { return None; }
        i -= 1;
    }
    None
}

/// Replaces `@query` (from `at` to `cursor`) by `@Name ` and returns the new text and cursor.
pub(crate) fn insert_mention(text: &str, at: usize, cursor: usize, m: &Mention) -> (String, usize) {
    let (a, b) = (byte_of(text, at), byte_of(text, cursor));
    let ins = format!("{} ", m.label());
    let mut out = String::from(&text[..a]);
    out.push_str(&ins);
    out.push_str(&text[b..]);
    (out, at + ins.chars().count())
}

/// Mentions that are still in the text (the user may have deleted the pill), each once, in text order.
pub(crate) fn live_mentions<'a>(text: &str, ms: &'a [Mention]) -> Vec<&'a Mention> {
    let mut v: Vec<(usize, &Mention)> = ms.iter().filter_map(|m| text.find(&m.label()).map(|p| (p, m))).collect();
    v.sort_by_key(|(p, _)| *p);
    v.into_iter().map(|(_, m)| m).collect()
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// The message's `object.content` HTML: escaped text, `<br>` for line breaks and `<spark-mention>` for every mention
/// (`data-object-type="person"` + `data-object-id`, or `groupMention` + `data-group-type="all"` for @all).
pub(crate) fn to_content_html(text: &str, ms: &[Mention]) -> String {
    let mut html = esc(text);
    for m in live_mentions(text, ms) {
        let tag = if m.all { format!("<spark-mention data-object-type=\"groupMention\" data-group-type=\"all\">{}</spark-mention>", esc(&m.name)) }
                  else { format!("<spark-mention data-object-type=\"person\" data-object-id=\"{}\">{}</spark-mention>", esc(&m.id), esc(&m.name)) };
        html = html.replacen(&esc(&m.label()), &tag, 1);
    }
    html.replace('\n', "<br>")
}

// Emoji `:name`.

/// `:name` trigger: the `:` must start the text or follow whitespace, the name is word characters; returns (char index of the `:`,
/// name) once the name has at least `min` characters.
pub(crate) fn emoji_query(text: &str, cursor: usize, min: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut i = cursor;
    while i > 0 && (chars[i - 1].is_alphanumeric() || "_+-".contains(chars[i - 1])) { i -= 1; }
    if i == 0 || chars[i - 1] != ':' { return None; }
    if i >= 2 && !chars[i - 2].is_whitespace() { return None; }
    let name: String = chars[i..cursor].iter().collect();
    (name.chars().count() >= min).then(|| (i - 1, name))
}

// Formatting.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Format { Bold, Italic, Underline, Strike, Link, NumberList, BulletList, CodeBlock, InlineCode, Quote, Heading(u8), Sub, Sup, Indent, Dedent }

/// Result of an edit: new text and the new selection (anchor, caret) in chars.
pub(crate) type Edit = (String, usize, usize);

fn wrap(text: &str, a: usize, b: usize, open: &str, close: &str) -> Edit {
    let (ab, bb) = (byte_of(text, a), byte_of(text, b));
    let (ol, cl) = (open.chars().count(), close.chars().count());
    let sel = &text[ab..bb];
    // Already wrapped around the selection: unwrap.
    if ab >= open.len() && text[..ab].ends_with(open) && text[bb..].starts_with(close) && !(open == "_" && text[..ab].ends_with("__")) {
        let mut out = String::from(&text[..ab - open.len()]);
        out.push_str(sel);
        out.push_str(&text[bb + close.len()..]);
        return (out, a - ol, b - ol);
    }
    // The selection includes its markers: unwrap.
    if sel.len() >= open.len() + close.len() && sel.starts_with(open) && sel.ends_with(close) && a != b {
        let inner = &sel[open.len()..sel.len() - close.len()];
        let out = format!("{}{}{}", &text[..ab], inner, &text[bb..]);
        return (out, a, b - ol - cl);
    }
    let out = format!("{}{}{}{}{}", &text[..ab], open, sel, close, &text[bb..]);
    if a == b { (out, a + ol, a + ol) } else { (out, a + ol, b + ol) }
}

/// Line range (byte) of the lines touched by the selection.
fn line_span(text: &str, a: usize, b: usize) -> (usize, usize) {
    let (ab, bb) = (byte_of(text, a), byte_of(text, b));
    let s = text[..ab].rfind('\n').map_or(0, |p| p + 1);
    let e = text[bb..].find('\n').map_or(text.len(), |p| bb + p);
    (s, e)
}

fn strip_prefix_of_line(line: &str) -> (&str, &str) {
    let t = line.trim_start();
    for p in ["### ", "## ", "# ", "> ", "* ", "- "] { if t.starts_with(p) { return (&line[..line.len() - t.len()], &t[p.len()..]); } }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && t[digits..].starts_with(". ") { return (&line[..line.len() - t.len()], &t[digits + 2..]); }
    (&line[..line.len() - t.len()], t)
}

/// Toggles a line prefix (`# `, `> `, `* `, `1. `) on every selected line; headings replace each other.
fn line_prefix(text: &str, a: usize, b: usize, kind: &str) -> Edit {
    let (s, e) = line_span(text, a, b);
    let lines: Vec<&str> = text[s..e].split('\n').collect();
    let has = |l: &str| match kind {
        "num" => { let t = l.trim_start(); t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(". ") }
        k => l.trim_start().starts_with(k),
    };
    let all_have = lines.iter().all(|l| has(l));
    let mut out_lines = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let (ind, body) = strip_prefix_of_line(l);
        let body_clean = if has(l) || kind.starts_with('#') { body } else { l.trim_start() };
        let body = if kind.starts_with('#') || has(l) { body_clean } else { body_clean };
        if all_have { out_lines.push(format!("{ind}{body}")); }
        else if kind == "num" { out_lines.push(format!("{ind}{}. {body}", i + 1)); }
        else { out_lines.push(format!("{ind}{kind}{body}")); }
    }
    let joined = out_lines.join("\n");
    let out = format!("{}{}{}", &text[..s], joined, &text[e..]);
    let end = char_of(&out, s + joined.len());
    (out, end, end)
}

/// One formatting command (toolbar button or shortcut) applied to `text` with the selection `a..b` (either order).
pub(crate) fn apply_format(text: &str, a: usize, b: usize, f: Format) -> Edit {
    let (a, b) = (a.min(b), a.max(b));
    match f {
        Format::Bold => wrap(text, a, b, "**", "**"),
        Format::Italic => wrap(text, a, b, "_", "_"),
        // Markdown has no underline: the HTML tag is kept in the text (rich-text mode sends it as <u>).
        Format::Underline => wrap(text, a, b, "<u>", "</u>"),
        Format::Strike => wrap(text, a, b, "~~", "~~"),
        Format::InlineCode => wrap(text, a, b, "`", "`"),
        Format::Sub => wrap(text, a, b, "<sub>", "</sub>"),
        Format::Sup => wrap(text, a, b, "<sup>", "</sup>"),
        Format::CodeBlock => {
            let (ab, bb) = (byte_of(text, a), byte_of(text, b));
            let out = format!("{}```\n{}\n```{}", &text[..ab], &text[ab..bb], &text[bb..]);
            let p = a + 4;
            (out, p, p + (b - a))
        }
        Format::Link => {
            let (ab, bb) = (byte_of(text, a), byte_of(text, b));
            let sel = &text[ab..bb];
            let label = if sel.is_empty() { "Linktext" } else { sel };
            let out = format!("{}[{}](Link){}", &text[..ab], label, &text[bb..]);
            let start = a + 1 + label.chars().count() + 2;
            (out, start, start + 4)
        }
        Format::Heading(n) => { let p = format!("{} ", "#".repeat(n as usize)); line_prefix(text, a, b, &p) }
        Format::Quote => line_prefix(text, a, b, "> "),
        Format::BulletList => line_prefix(text, a, b, "* "),
        Format::NumberList => line_prefix(text, a, b, "num"),
        Format::Indent | Format::Dedent => {
            let (s, e) = line_span(text, a, b);
            let lines: Vec<String> = text[s..e].split('\n').map(|l| if f == Format::Indent { format!("  {l}") } else { l.strip_prefix("  ").or_else(|| l.strip_prefix('\t')).or_else(|| l.strip_prefix(' ')).unwrap_or(l).to_string() }).collect();
            let joined = lines.join("\n");
            let out = format!("{}{}{}", &text[..s], joined, &text[e..]);
            let c = char_of(&out, s + joined.len());
            (out, c, c)
        }
    }
}

/// Shift+Enter in markdown mode: a new line that continues a list (`1. x` -> `2. `, `* x` -> `* `); on an empty list item it
/// leaves the list instead (the marker is removed). Plain text: just a newline. Returns the new text and cursor.
pub(crate) fn newline(text: &str, cursor: usize, markdown: bool) -> (String, usize) {
    let cb = byte_of(text, cursor);
    let ls = text[..cb].rfind('\n').map_or(0, |p| p + 1);
    let line = &text[ls..cb];
    if markdown {
        let (ind, body) = strip_prefix_of_line(line);
        let marker = &line[ind.len()..line.len() - body.len()];
        let is_list = marker.starts_with("* ") || marker.starts_with("- ") || marker.chars().next().is_some_and(|c| c.is_ascii_digit());
        if is_list {
            if body.trim().is_empty() {
                let out = format!("{}{}{}", &text[..ls], ind, &text[cb..]);
                let c = char_of(&out, ls + ind.len());
                return (out, c);
            }
            let next = if let Some(n) = marker.trim_end_matches(". ").parse::<u32>().ok().filter(|_| marker.ends_with(". ")) { format!("{}. ", n + 1) } else { marker.to_string() };
            let ins = format!("\n{ind}{next}");
            let out = format!("{}{}{}", &text[..cb], ins, &text[cb..]);
            return (out, cursor + ins.chars().count());
        }
    }
    let out = format!("{}\n{}", &text[..cb], &text[cb..]);
    (out, cursor + 1)
}

/// Backspace on an empty list item dedents/leaves the list (markdown mode): returns the edit when the caret sits right behind a bare marker.
pub(crate) fn backspace_empty_item(text: &str, cursor: usize) -> Option<(String, usize)> {
    let cb = byte_of(text, cursor);
    let ls = text[..cb].rfind('\n').map_or(0, |p| p + 1);
    let line = &text[ls..cb];
    let (ind, body) = strip_prefix_of_line(line);
    if !body.is_empty() || ind.len() == line.len() { return None; }
    let out = format!("{}{}{}", &text[..ls], ind, &text[cb..]);
    Some((out.clone(), char_of(&out, ls + ind.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoticons_become_emoji_before_the_space() {
        let (t, c) = replace_emoticon_before_space("hi :) ", 6).unwrap();
        assert_eq!(t, "hi 🙂 "); assert_eq!(c, 5);
        assert!(replace_emoticon_before_space("http://x ", 9).is_none());
        assert_eq!(emoticon_for("XD"), Some("😆")); assert_eq!(emoticon_for("xd"), Some("😆")); assert_eq!(emoticon_for(":'("), Some("😢"));
        assert_eq!(emoticon_for("<3"), None);
    }

    #[test]
    fn send_parses_every_word_once() {
        assert_eq!(parse_emoticons("ok :) und ;)\nxd -_-"), "ok 🙂 und 😉\n😆 😑");
        assert_eq!(parse_emoticons("https://a.b/:)"), "https://a.b/:)");
    }

    #[test]
    fn mention_trigger_and_token() {
        assert_eq!(mention_query("hallo @To", 9), Some((6, "To".into())));
        assert_eq!(mention_query("mail@x", 6), None);
        assert_eq!(mention_query("@", 1), Some((0, String::new())));
        let m = Mention { id: "p1".into(), name: "Mara Lind".into(), all: false };
        let (t, c) = insert_mention("hallo @To", 6, 9, &m);
        assert_eq!(t, "hallo @Mara Lind "); assert_eq!(c, 17);
        let html = to_content_html("hallo @Mara Lind <b>\nok", &[m.clone()]);
        assert_eq!(html, "hallo <spark-mention data-object-type=\"person\" data-object-id=\"p1\">Mara Lind</spark-mention> &lt;b&gt;<br>ok");
        assert!(live_mentions("nichts", &[m]).is_empty());
        let all = Mention { id: String::new(), name: "all".into(), all: true };
        assert!(to_content_html("@all hi", &[all]).contains("groupMention"));
    }

    #[test]
    fn emoji_trigger() {
        assert_eq!(emoji_query("hi :so", 6, 2), Some((3, "so".into())));
        assert_eq!(emoji_query("hi :s", 5, 2), None);
        assert_eq!(emoji_query("um 10:30", 8, 2), None);
        assert_eq!(emoji_query(":)", 2, 2), None);
    }

    #[test]
    fn markdown_toggles() {
        assert_eq!(apply_format("ab cd", 3, 5, Format::Bold), ("ab **cd**".into(), 5, 7));
        assert_eq!(apply_format("ab **cd**", 5, 7, Format::Bold), ("ab cd".into(), 3, 5));
        assert_eq!(apply_format("", 0, 0, Format::Italic), ("__".into(), 1, 1));
        assert_eq!(apply_format("x", 0, 1, Format::Heading(2)).0, "## x");
        assert_eq!(apply_format("## x", 0, 4, Format::Heading(2)).0, "x");
        assert_eq!(apply_format("a\nb", 0, 3, Format::NumberList).0, "1. a\n2. b");
        assert_eq!(apply_format("a\nb", 0, 3, Format::BulletList).0, "* a\n* b");
        assert_eq!(apply_format("code", 0, 4, Format::CodeBlock).0, "```\ncode\n```");
        assert_eq!(apply_format("w", 0, 1, Format::Link).0, "[w](Link)");
    }

    #[test]
    fn shift_enter_continues_lists() {
        assert_eq!(newline("1. eins", 7, true), ("1. eins\n2. ".into(), 11));
        assert_eq!(newline("* a", 3, true), ("* a\n* ".into(), 6));
        assert_eq!(newline("* ", 2, true), ("".into(), 0));
        assert_eq!(newline("plain", 5, true), ("plain\n".into(), 6));
        assert_eq!(newline("* a", 3, false), ("* a\n".into(), 4));
    }
}
