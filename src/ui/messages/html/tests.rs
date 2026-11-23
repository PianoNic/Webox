use super::*;

fn parse(h: &str) -> Parsed { html_job(h, &mc(&crate::dark()), "", 400.0, false).expect("parsed") }
fn fmt_of<'a>(p: &'a Parsed, word: &str) -> &'a egui::TextFormat {
    let at = p.job.text.find(word).expect(word);
    &p.job.sections.iter().find(|s| s.byte_range.start <= at && at < s.byte_range.end).unwrap().format
}

#[test]
fn users_example_keeps_link_target_and_line_break() {
    let url = "https://dev.azure.com/contoso/Contoso%20Apps/_git/Portal/pullrequest/4711";
    let p = parse(&format!("<a href=\"{url}\">{url}</a><br/>Bitte bis Freitag anschauen."));
    let lines: Vec<&str> = p.job.text.lines().collect();
    assert_eq!(lines, [url, "Bitte bis Freitag anschauen."]);
    assert_eq!(p.links.len(), 1);
    assert_eq!(p.links[0].1, url); // %20 is never decoded
    assert_eq!(&p.job.text[p.links[0].0.clone()], url); // the link does not run into the second sentence
}

#[test]
fn bare_urls_in_text_nodes_are_linked_but_not_inside_anchors() {
    let p = parse("see https://x.example/a, ok <a href=\"https://y.example/q%20r\">label</a> and www.z.example/p.");
    let t: Vec<&str> = p.links.iter().map(|l| l.1.as_str()).collect();
    assert_eq!(t, ["https://x.example/a", "https://y.example/q%20r", "www.z.example/p"]);
    assert_eq!(&p.job.text[p.links[1].0.clone()], "label");
}

#[test]
fn mentions_use_the_mention_colours() {
    let c = mc(&crate::dark());
    let p = html_job("hi <spark-mention data-object-type=\"person\" data-object-id=\"abc-1\">Me</spark-mention> and <spark-mention data-object-type=\"person\" data-object-id=\"zzz\">Other</spark-mention> <spark-mention data-object-type=\"groupMention\" data-group-type=\"all\">all</spark-mention>", &c, "abc-1", 400.0, false).unwrap();
    assert_eq!(fmt_of(&p, "Me").color, c.accent);
    assert_eq!(fmt_of(&p, "Other").color, c.mention_other);
    assert_eq!(fmt_of(&p, "all").color, c.accent);
    assert_eq!(fmt_of(&p, "hi").color, c.body);
}

#[test]
fn nested_formatting() {
    let p = parse("<b>a <i>b</i></b> <u>u</u> <s>s</s> <code>c</code>");
    assert_eq!(p.job.text, "a b u s c");
    assert!(fmt_of(&p, "a").font_id.family == egui::FontFamily::Name("semibold".into()));
    let b = fmt_of(&p, "b");
    assert!(b.italics && b.font_id.family == egui::FontFamily::Name("semibold".into()));
    assert!(fmt_of(&p, "u").underline.width > 0.0);
    assert!(fmt_of(&p, "s").strikethrough.width > 0.0);
    assert!(fmt_of(&p, "c").font_id.family == egui::FontFamily::Monospace);
    assert!(!fmt_of(&p, "s").italics);
}

#[test]
fn entities_are_decoded_in_text_and_attributes() {
    let p = parse("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#39; f&nbsp;g &#x41;&#66; &bogus; <a href=\"https://q.example/?a=1&amp;b=2\">q</a>");
    assert_eq!(p.job.text, "a & b <c> \"d\" 'e' f\u{a0}g AB &bogus; q");
    assert_eq!(p.links[0].1, "https://q.example/?a=1&b=2");
}

#[test]
fn blocks_lists_and_quotes() {
    assert_eq!(parse("<p>one</p><p>two</p>").job.text, "one\ntwo");
    assert_eq!(parse("a<br>b<br><br>c").job.text, "a\nb\n\nc");
    assert_eq!(parse("a<br>").job.text, "a"); // a trailing break adds no empty line
    let ul = parse("<ul><li>x</li><li>y</li></ul>");
    assert_eq!(ul.job.text.lines().count(), 2);
    assert!(ul.job.text.contains("• x") && ul.job.text.contains("• y"));
    let ol = parse("<ol><li>x</li><li>y</li></ol>");
    assert!(ol.job.text.contains("1. x") && ol.job.text.contains("2. y"));
    let q = parse("<blockquote>quoted</blockquote>after");
    assert_eq!(q.quotes.len(), 1);
    assert_eq!(q.job.text.lines().count(), 2);
    assert!(q.job.text.lines().next().unwrap().ends_with("quoted"));
    let pre = parse("<pre>l1\n  l2\n</pre>");
    assert_eq!(pre.job.text, "l1\n  l2");
}

#[test]
fn reply_quotes_get_a_header_with_the_jump_link() {
    let p = parse("<blockquote><span>Timo Steiner</span><br/>Build ist durch</blockquote>Immerhin etwas");
    let lines: Vec<&str> = p.job.text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].ends_with("Timo Steiner \u{2022} Zur Originalnachricht springen"), "{lines:?}");
    assert!(lines[1].ends_with("Build ist durch"));
    assert_eq!(lines[2], "Immerhin etwas");
    assert_eq!(p.reply_quote.as_deref(), Some("Build ist durch"));
    assert_eq!(p.quotes.len(), 1);
    let j = p.links.iter().find(|l| l.1 == JUMP).expect("jump link");
    assert_eq!(&p.job.text[j.0.clone()], "Zur Originalnachricht springen");
    // A quote without author line is an ordinary quote.
    let q = parse("<blockquote>nur Text</blockquote>x");
    assert!(q.reply_quote.is_none() && q.links.is_empty());
}

#[test]
fn headings_use_qts_scales_and_emoji_grow_with_them() {
    for (tag, k) in [("h1", 2.0), ("h2", 1.5), ("h3", 1.2), ("h4", 1.0), ("h5", 0.8), ("h6", 1.0)] {
        let p = parse(&format!("<{tag}>Titel</{tag}>text"));
        let f = fmt_of(&p, "Titel");
        assert!((f.font_id.size - BODY * k).abs() < 1e-3, "{tag}");
        // h1..h5 are bold, h6 keeps the normal weight.
        assert_eq!(f.font_id.family == egui::FontFamily::Name("semibold".into()), tag != "h6", "{tag} weight");
        assert_eq!(p.job.text.lines().count(), 2);
        assert!((fmt_of(&p, "text").font_id.size - BODY).abs() < 1e-3);
    }
    // "# 😀" arrives as <h1>😀</h1>: the picture is drawn at twice the size, a plain emoji stays at the inline size.
    let p = parse("<h1>\u{1F600}</h1>\u{1F600}");
    assert_eq!(p.runs.len(), 2);
    assert!((p.runs[0].size - EMOJI_SIZE * 2.0).abs() < 1e-3);
    assert!((p.runs[1].size - EMOJI_SIZE).abs() < 1e-3);
}

#[test]
fn broken_or_empty_html_falls_back() {
    let c = mc(&crate::dark());
    assert!(html_job("", &c, "", 400.0, false).is_none());
    assert!(html_job("<p> </p><br>", &c, "", 400.0, false).is_none());
    // Unbalanced / unknown markup still shows its text.
    assert_eq!(parse("<foo>a<b>b</foo> c < d").job.text, "ab c < d");
}

#[test]
fn many_unterminated_tags_stay_text() {
    // Every `<` without a `>` after it used to rescan to the end of the message.
    let s = "x <y".repeat(50_000);
    assert_eq!(parse(&s).job.text, s);
    // A `>` at the very end that no `<` can use as its tag end (an invalid tag name): still all text.
    let s = format!("{}>", "<1".repeat(20_000));
    assert_eq!(parse(&s).job.text, s);
    // Quotes hide a `>` from the `<` before them, not from a later one.
    assert_eq!(parse("a < \" <b>b</b>").job.text, "a < \" b");
    assert_eq!(fmt_of(&parse("a < \" <b>bold</b>"), "bold").font_id.family, egui::FontFamily::Name("semibold".into()));
}

#[test]
fn deep_nesting_is_capped() {
    let n = 2_000;
    let q = parse(&format!("{}x{}after", "<blockquote>".repeat(n), "</blockquote>".repeat(n)));
    let first = q.job.text.lines().next().unwrap();
    assert!(first.ends_with('x') && first.chars().filter(|&c| c == '\u{2002}').count() <= MAX_DEPTH, "{} indents", first.chars().count());
    assert!(q.job.text.ends_with("after"));
    let l = parse(&format!("{}<li>x</li>{}", "<ul>".repeat(n), "</ul>".repeat(n)));
    assert!(l.job.text.ends_with('x') && l.job.text.chars().filter(|&c| c == '\u{2003}').count() <= MAX_DEPTH, "{:?}", l.job.text.len());
    // Unclosed paragraphs beyond the cap still start new lines.
    let p = parse(&"<p>z".repeat(100));
    assert_eq!(p.job.text.lines().count(), 100);
    // Formatting below the cap still works.
    let b = parse(&format!("{}<b>fett</b>", "<span>".repeat(10)));
    assert_eq!(fmt_of(&b, "fett").font_id.family, egui::FontFamily::Name("semibold".into()));
}
