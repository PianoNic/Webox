//! Markdown (the composer's markdown mode, and the markers its rich-text toolbar inserts) → the HTML Webex expects
//! as `object.content`, plus the plain-text rendition sent as `object.displayName`.
//!
//! Wire format:
//!   * `# 😀` is sent as `content = <h1>😀</h1>` and `displayName = 😀` (markers gone); paragraphs are `<p>…</p>`,
//!     bold `<strong>`, `<hr/>`, lists `<ul><li>` / `<ol><li>` (`<ol start="3">` when it doesn't begin at 1), links `<a href="…">`
//!     (bare URLs are linked too, text = URL), line breaks `<br>`; mentions
//!     `<spark-mention data-object-type="person" data-object-id="<uuid>" data-mention-origin="client">Name</spark-mention>` (no `@`
//!     inside) with `object.mentions.items[{id: <uuid>, objectType: "person"}]`.
//!   * `displayName` is the markup's text content: tags are stripped, `<br>` / block ends add no separator.
//!   * a plain message has no `content` at all, so `html` stays empty unless something is formatted.
//!   * italic `<em>`, inline `<code>`, fenced `<pre><code class="language-x">`, `<blockquote>`, strike `<s>`;
//!     `<u>`, `<sub>`, `<sup>` (inserted by the toolbar) pass through. Links typed without a scheme get no prefix.
//!
//! Markdown syntax: `**Bold**`, `_Italic_`, `> Blockquote`, `1. First item`, `* Bullet point 1`, `# Heading 1`.. ,
//! `--- Horizontal line`, `[Link Text](link)`, ```` ``` language ```` fenced code, `` `Unformatted text` ``.

/// A person (or @all) mentioned in the text as `@name`; `id` is the Webex uuid (ignored for `all`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MentionRef { pub id: String, pub name: String, pub all: bool }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rendered {
    /// `object.content`; empty when the message needs none (nothing formatted, no mention).
    pub html: String,
    /// `object.displayName`: the typed text itself when there is no `html`, else the markup's text content.
    pub plain: String,
    /// The mentions that ended up as `<spark-mention>` in `html` (for `object.mentions`); ones inside code stay text.
    pub mentions: Vec<MentionRef>,
}

const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }
fn esc_attr(s: &str) -> String { esc(s).replace('"', "&quot;") }

/// `markdown`: interpret the markdown syntax; otherwise the text is literal (only mentions make it HTML).
pub fn render(text: &str, markdown: bool, mentions: &[MentionRef]) -> Rendered {
    // Mentions first: "@Name" → a marker that survives the markdown pass (first occurrence of each, like the composer's pills).
    let mut src = text.to_string();
    let mut used: Vec<&MentionRef> = vec![];
    for m in mentions {
        let label = format!("@{}", m.name);
        if let Some(p) = src.find(&label) {
            src.replace_range(p..p + label.len(), &format!("{OPEN}{}{CLOSE}", used.len()));
            used.push(m);
        }
    }
    let mut cx = Cx { used: vec![false; used.len()], mentions: used, markup: false, md: markdown };
    let (html, plain) = if markdown { blocks(&src, &mut cx) } else { paragraphs(&src, &mut cx) };
    let mentions: Vec<MentionRef> = cx.mentions.iter().zip(&cx.used).filter(|(_, u)| **u).map(|(m, _)| (*m).clone()).collect();
    if !cx.markup { return Rendered { html: String::new(), plain: put_mentions_plain(&src, &cx.mentions), mentions } }
    Rendered { html, plain, mentions }
}

fn put_mentions_plain(src: &str, ms: &[&MentionRef]) -> String {
    let mut out = String::new();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        if c == OPEN {
            let n: String = it.by_ref().take_while(|c| *c != CLOSE).collect();
            if let Some(m) = n.parse::<usize>().ok().and_then(|i| ms.get(i)) { out.push('@'); out.push_str(&m.name); }
        } else { out.push(c); }
    }
    out
}

struct Cx<'a> { mentions: Vec<&'a MentionRef>, used: Vec<bool>, markup: bool, md: bool }

/// @all: `data-object-type="groupMention"` + `data-group-type`; the inner text must be a single word (kept as typed).
fn mention_html(m: &MentionRef) -> String {
    if m.all { format!("<spark-mention data-object-type=\"groupMention\" data-group-type=\"all\">{}</spark-mention>", esc(&m.name)) }
    else { format!("<spark-mention data-object-type=\"person\" data-object-id=\"{}\" data-mention-origin=\"client\">{}</spark-mention>", esc_attr(&m.id), esc(&m.name)) }
}

// Blocks.

/// Literal text: blank-line separated paragraphs, single newlines are `<br/>`; only mention markers become markup.
fn paragraphs(src: &str, cx: &mut Cx) -> (String, String) {
    let (mut html, mut plain) = (String::new(), String::new());
    for chunk in src.split("\n\n") {
        let lines: Vec<String> = chunk.split('\n').map(|l| { let (h, p) = inline(l, cx); plain.push_str(&p); h }).collect();
        html.push_str(&format!("<p>{}</p>", lines.join("<br/>")));
    }
    (html, plain)
}

fn is_hr(l: &str) -> bool { let t = l.trim(); t.len() >= 3 && t.chars().all(|c| c == '-') }
fn heading(l: &str) -> Option<(usize, &str)> {
    let n = l.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&n).then(|| l[n..].strip_prefix(' ')).flatten().map(|r| (n, r.trim()))
}
fn ordered(l: &str) -> Option<(u32, &str)> {
    let t = l.trim_start();
    let d = t.chars().take_while(|c| c.is_ascii_digit()).count();
    (d > 0 && t[d..].starts_with(". ")).then(|| (t[..d].parse().unwrap_or(1), t[d + 2..].trim()))
}
fn bullet(l: &str) -> Option<&str> { let t = l.trim_start(); ["* ", "- ", "+ "].iter().find_map(|p| t.strip_prefix(p)).map(str::trim) }
fn quote(l: &str) -> Option<&str> { l.trim_start().strip_prefix('>').map(|r| r.strip_prefix(' ').unwrap_or(r)) }
fn fence(l: &str) -> Option<&str> { l.trim_start().strip_prefix("```").map(str::trim) }
fn starts_block(l: &str) -> bool { l.trim().is_empty() || heading(l).is_some() || is_hr(l) || ordered(l).is_some() || bullet(l).is_some() || quote(l).is_some() || fence(l).is_some() }

fn blocks(src: &str, cx: &mut Cx) -> (String, String) {
    let lines: Vec<&str> = src.split('\n').collect();
    let (mut html, mut plain) = (String::new(), String::new());
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.trim().is_empty() { i += 1; continue }
        if let Some(lang) = fence(l) {
            let mut code = vec![];
            i += 1;
            while i < lines.len() && fence(lines[i]) != Some("") { code.push(lines[i]); i += 1; }
            i += 1; // the closing fence
            let body = put_mentions_plain(&code.join("\n"), &cx.mentions);
            let class = if lang.is_empty() { String::new() } else { format!(" class=\"language-{}\"", esc_attr(lang)) };
            html.push_str(&format!("<pre><code{class}>{}</code></pre>", esc(&body)));
            plain.push_str(&body);
            cx.markup = true;
        } else if let Some((n, t)) = heading(l) {
            let (h, p) = inline(t, cx);
            html.push_str(&format!("<h{n}>{h}</h{n}>")); plain.push_str(&p); cx.markup = true; i += 1;
        } else if is_hr(l) {
            html.push_str("<hr/>"); cx.markup = true; i += 1;
        } else if quote(l).is_some() {
            let mut parts = vec![];
            while i < lines.len() { let Some(q) = quote(lines[i]) else { break }; let (h, p) = inline(q, cx); plain.push_str(&p); parts.push(h); i += 1; }
            html.push_str(&format!("<blockquote>{}</blockquote>", parts.join("<br/>"))); cx.markup = true;
        } else if let Some((first, _)) = ordered(l) {
            let mut items = vec![];
            while i < lines.len() { let Some((_, t)) = ordered(lines[i]) else { break }; let (h, p) = inline(t, cx); plain.push_str(&p); items.push(format!("<li>{h}</li>")); i += 1; }
            let start = if first == 1 { String::new() } else { format!(" start=\"{first}\"") };
            html.push_str(&format!("<ol{start}>{}</ol>", items.concat())); cx.markup = true;
        } else if bullet(l).is_some() {
            let mut items = vec![];
            while i < lines.len() { let Some(t) = bullet(lines[i]) else { break }; let (h, p) = inline(t, cx); plain.push_str(&p); items.push(format!("<li>{h}</li>")); i += 1; }
            html.push_str(&format!("<ul>{}</ul>", items.concat())); cx.markup = true;
        } else {
            let mut parts = vec![];
            while i < lines.len() && (parts.is_empty() || !starts_block(lines[i])) { let (h, p) = inline(lines[i], cx); plain.push_str(&p); parts.push(h); i += 1; }
            html.push_str(&format!("<p>{}</p>", parts.join("<br/>")));
        }
    }
    (html, plain)
}

// Inline.

fn find_close(chars: &[char], from: usize, pat: &[char]) -> Option<usize> {
    let mut j = from;
    while j + pat.len() <= chars.len() { if chars[j..j + pat.len()] == *pat { return Some(j); } j += 1; }
    None
}

fn url_end(chars: &[char], from: usize) -> usize {
    let mut e = from;
    while e < chars.len() && !chars[e].is_whitespace() && chars[e] != '<' && chars[e] != OPEN { e += 1; }
    while e > from && ".,;:!?)]}'\"".contains(chars[e - 1]) { e -= 1; }
    e
}

/// (html, plain) of one line of text.
fn inline(s: &str, cx: &mut Cx) -> (String, String) {
    let ch: Vec<char> = s.chars().collect();
    let (mut h, mut p) = (String::new(), String::new());
    let mut i = 0;
    let word = |c: char| c.is_alphanumeric();
    while i < ch.len() {
        let c = ch[i];
        let prev = if i > 0 { Some(ch[i - 1]) } else { None };
        // Mention marker.
        if c == OPEN {
            if let Some(e) = ch[i..].iter().position(|c| *c == CLOSE) {
                let n: String = ch[i + 1..i + e].iter().collect();
                if let Some((k, m)) = n.parse::<usize>().ok().and_then(|k| cx.mentions.get(k).map(|m| (k, *m))) {
                    h.push_str(&mention_html(m)); p.push_str(&m.name); cx.markup = true; cx.used[k] = true; i += e + 1; continue;
                }
            }
        }
        if cx.md {
            // Inline code.
            if c == '`' {
                if let Some(e) = find_close(&ch, i + 1, &['`']).filter(|e| *e > i + 1) {
                    let t: String = put_mentions_plain(&ch[i + 1..e].iter().collect::<String>(), &cx.mentions);
                    h.push_str(&format!("<code>{}</code>", esc(&t))); p.push_str(&t); cx.markup = true; i = e + 1; continue;
                }
            }
            // **bold** and ~~strike~~.
            let mut consumed = false;
            for (pat, tag) in [("**", "strong"), ("~~", "s")] {
                let pc: Vec<char> = pat.chars().collect();
                if ch[i..].starts_with(&pc) {
                    if let Some(e) = find_close(&ch, i + 2, &pc).filter(|e| *e > i + 2) {
                        let inner: String = ch[i + 2..e].iter().collect();
                        let (ih, ip) = inline(&inner, cx);
                        h.push_str(&format!("<{tag}>{ih}</{tag}>")); p.push_str(&ip); cx.markup = true; i = e + 2;
                        consumed = true;
                        break;
                    }
                }
            }
            if consumed { continue }
            // _italic_ / *italic*: the opener may not touch a word on its left, the closer not on its right.
            if (c == '_' || c == '*') && !prev.is_some_and(word) && ch.get(i + 1).is_some_and(|n| !n.is_whitespace() && *n != c) {
                let close = (i + 2..ch.len()).find(|&j| ch[j] == c && !ch[j - 1].is_whitespace() && !ch.get(j + 1).is_some_and(|n| word(*n)));
                if let Some(e) = close {
                    let inner: String = ch[i + 1..e].iter().collect();
                    let (ih, ip) = inline(&inner, cx);
                    h.push_str(&format!("<em>{ih}</em>")); p.push_str(&ip); cx.markup = true; i = e + 1; continue;
                }
            }
            // [text](url).
            if c == '[' {
                if let Some(mid) = find_close(&ch, i + 1, &[']', '(']) {
                    if let Some(end) = find_close(&ch, mid + 2, &[')']) {
                        let label: String = ch[i + 1..mid].iter().collect();
                        let url: String = ch[mid + 2..end].iter().collect::<String>().trim().to_string();
                        if !label.is_empty() && !url.is_empty() && !url.contains(char::is_whitespace) {
                            let href = if url.contains("://") || url.starts_with("mailto:") { url } else { format!("https://{url}") };
                            let (ih, ip) = inline(&label, cx);
                            h.push_str(&format!("<a href=\"{}\">{ih}</a>", esc_attr(&href))); p.push_str(&ip); cx.markup = true; i = end + 1; continue;
                        }
                    }
                }
            }
            // Tags the toolbar inserts.
            let mut tagged = false;
            for t in ["<u>", "</u>", "<sub>", "</sub>", "<sup>", "</sup>"] {
                let tc: Vec<char> = t.chars().collect();
                if ch[i..].starts_with(&tc) { h.push_str(t); cx.markup = true; i += tc.len(); tagged = true; break; }
            }
            if tagged { continue }
            // Bare URL → link.
            if (c == 'h') && !prev.is_some_and(word) && (ch[i..].starts_with(&['h', 't', 't', 'p', ':', '/', '/']) || ch[i..].starts_with(&['h', 't', 't', 'p', 's', ':', '/', '/'])) {
                let e = url_end(&ch, i);
                if e > i + 8 {
                    let u: String = ch[i..e].iter().collect();
                    h.push_str(&format!("<a href=\"{}\">{}</a>", esc_attr(&u), esc(&u))); p.push_str(&u); cx.markup = true; i = e; continue;
                }
            }
        }
        // A char that opened a pair above but found no partner, or ordinary text.
        let t = c.to_string();
        h.push_str(&esc(&t)); p.push(c);
        i += 1;
    }
    (h, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn md(t: &str) -> Rendered { render(t, true, &[]) }
    fn html(t: &str) -> String { md(t).html }

    #[test]
    fn heading_with_emoji() {
        // "# 😀" must not arrive as literal text.
        let r = md("# 😀");
        assert_eq!(r.html, "<h1>😀</h1>");
        assert_eq!(r.plain, "😀");
        assert_eq!(html("###### x"), "<h6>x</h6>");
        assert_eq!(html("####### x"), ""); // seven # is no heading
        assert_eq!(html("#nospace"), "");
    }

    #[test]
    fn plain_text_has_no_content() {
        for t in ["hello", "a\nb", "a\n\nb", "snake_case_name and 2*3*4", "price 5 * 3", "a_b c_d"] {
            let r = md(t);
            assert_eq!((r.html.as_str(), r.plain.as_str()), ("", t), "{t}");
        }
        // Markdown off: the syntax is literal.
        let r = render("# **x**", false, &[]);
        assert_eq!((r.html.as_str(), r.plain.as_str()), ("", "# **x**"));
    }

    #[test]
    fn inline_formats() {
        assert_eq!(html("a **b** c"), "<p>a <strong>b</strong> c</p>");
        assert_eq!(html("_it_ and *it2*"), "<p><em>it</em> and <em>it2</em></p>");
        assert_eq!(html("~~gone~~"), "<p><s>gone</s></p>");
        assert_eq!(html("use `a<b` here"), "<p>use <code>a&lt;b</code> here</p>");
        assert_eq!(html("**_both_**"), "<p><strong><em>both</em></strong></p>");
        assert_eq!(html("<u>x</u> <sub>2</sub>"), "<p><u>x</u> <sub>2</sub></p>"); // toolbar tags
        assert_eq!(html("1 < 2 & <script>"), ""); // other tags are plain text, escaped only if html is sent
        assert_eq!(html("**b** <script>"), "<p><strong>b</strong> &lt;script&gt;</p>");
        assert_eq!(md("a **b** c").plain, "a b c");
    }

    #[test]
    fn links() {
        assert_eq!(html("[Cisco](www.cisco.com)"), "<p><a href=\"https://www.cisco.com\">Cisco</a></p>");
        assert_eq!(html("[x](https://a.b/c?d=1&e=2)"), "<p><a href=\"https://a.b/c?d=1&amp;e=2\">x</a></p>");
        assert_eq!(html("see https://gitlab.com/a/b."), "<p>see <a href=\"https://gitlab.com/a/b\">https://gitlab.com/a/b</a>.</p>");
        assert_eq!(md("see https://x.org/a").plain, "see https://x.org/a");
        assert_eq!(html("[broken](no close"), "");
    }

    #[test]
    fn blocks_and_lists() {
        assert_eq!(html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>");
        assert_eq!(html("3. a\n4. b"), "<ol start=\"3\"><li>a</li><li>b</li></ol>");
        assert_eq!(html("* a\n* b **x**"), "<ul><li>a</li><li>b <strong>x</strong></li></ul>");
        assert_eq!(html("> q1\n> q2"), "<blockquote>q1<br/>q2</blockquote>");
        assert_eq!(html("---"), "<hr/>");
        assert_eq!(html("intro\n# T\ntext"), "<p>intro</p><h1>T</h1><p>text</p>");
        assert_eq!(html("a **b**\nc"), "<p>a <strong>b</strong><br/>c</p>");
        assert_eq!(md("# T\ntext").plain, "Ttext"); // text content, no separators (as displayName)
    }

    #[test]
    fn code_blocks() {
        assert_eq!(html("```\nlet a = 1;\n```"), "<pre><code>let a = 1;</code></pre>");
        assert_eq!(html("``` html\n<h1>Hello</h1>\n```"), "<pre><code class=\"language-html\">&lt;h1&gt;Hello&lt;/h1&gt;</code></pre>");
        // Markers inside code are not interpreted; an unclosed fence runs to the end.
        assert_eq!(html("```\n# not a heading **x**"), "<pre><code># not a heading **x**</code></pre>");
    }

    #[test]
    fn mentions() {
        let ms = [MentionRef { id: "11111111-2222-3333-4444-555555555555".into(), name: "Mara Lind".into(), all: false }];
        let r = render("Hallo @Mara Lind bitte prüfen", false, &ms);
        assert_eq!(r.html, "<p>Hallo <spark-mention data-object-type=\"person\" data-object-id=\"11111111-2222-3333-4444-555555555555\" data-mention-origin=\"client\">Mara Lind</spark-mention> bitte prüfen</p>");
        assert_eq!(r.plain, "Hallo Mara Lind bitte prüfen"); // the displayName has no @ (as in real messages)
        // A mention whose text was edited away is just text.
        let r = render("Hallo Mara", false, &ms);
        assert_eq!((r.html.as_str(), r.plain.as_str()), ("", "Hallo Mara"));
        // @all
        let all = [MentionRef { id: String::new(), name: "all".into(), all: true }];
        assert_eq!(render("@all lunch", true, &all).html, "<p><spark-mention data-object-type=\"groupMention\" data-group-type=\"all\">all</spark-mention> lunch</p>");
        // Together with markdown, and a name containing markdown characters stays intact.
        let odd = [MentionRef { id: "u".into(), name: "A_B".into(), all: false }];
        let r = render("**hi** @A_B", true, &odd);
        assert!(r.html.starts_with("<p><strong>hi</strong> <spark-mention") && r.html.contains(">A_B</spark-mention>"), "{}", r.html);
    }

    #[test]
    fn literal_mode_keeps_line_breaks() {
        let ms = [MentionRef { id: "u".into(), name: "X".into(), all: false }];
        assert_eq!(render("a\nb @X\n\nc <b>", false, &ms).html.replace(" data-mention-origin=\"client\"", ""),
            "<p>a<br/>b <spark-mention data-object-type=\"person\" data-object-id=\"u\">X</spark-mention></p><p>c &lt;b&gt;</p>");
    }
}
