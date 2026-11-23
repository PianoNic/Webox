use super::*;

fn links(text: &str) -> Vec<String> { body_job(text, &mc(&crate::dark()), "Alex Muster", 400.0, false).1.into_iter().map(|l| l.1).collect() }

#[test]
fn urls_become_links_without_trailing_punctuation() {
    assert_eq!(links("siehe https://example.org"), ["https://example.org"]);
    assert_eq!(links("Doku: https://example.com/a/b?x=1, danke."), ["https://example.com/a/b?x=1"]);
    assert_eq!(links("(https://example.com/x) und www.example.org/status."), ["https://example.com/x", "www.example.org/status"]);
    assert_eq!(links("https://en.wikipedia.org/wiki/Rust_(language)"), ["https://en.wikipedia.org/wiki/Rust_(language)"]);
    assert!(links("kein Link: foo.bar und ftp://x").is_empty());
}

#[test]
fn urls_end_at_line_breaks() {
    assert_eq!(links("https://dev.azure.com/x/pullrequest/4711\nBitte anschauen."), ["https://dev.azure.com/x/pullrequest/4711"]);
    assert_eq!(links("a\nhttps://one.example/x.\nhttps://two.example/y!\n"), ["https://one.example/x", "https://two.example/y"]);
    assert_eq!(links("siehe\r\nhttps://one.example/x"), ["https://one.example/x"]);
}

#[test]
fn hard_line_breaks_survive() {
    assert_eq!(normalize_breaks("a\r\nb\rc<br>d\u{2028}e"), "a\nb\nc\nd\ne");
    let (job, _, _) = body_job("https://x.example/1\nzweite Zeile", &mc(&crate::dark()), "", 400.0, false);
    assert!(job.text.contains('\n'));
    assert_eq!(job.text.lines().count(), 2);
}

#[test]
fn selection_helpers() {
    assert_eq!(word_range("hallo welt", 1), (0, 5));
    assert_eq!(word_range("hallo welt", 5), (5, 6)); // the space
    assert_eq!(paragraph_range("eins\nzwei drei\nvier", 7), (5, 14));
    let runs = [EmojiRun { char_idx: 2, picture: None, keycap: None, orig: "🫠".into(), size: EMOJI_SIZE }];
    let text = format!("a {}b", emoji_text::PLACEHOLDER);
    assert_eq!(selected_text(&text, &runs, &[], 0, 5), "a 🫠b");
    let lt = "see label now";
    assert_eq!(selected_text(lt, &[], &[(4..9, "https://x.example/a%20b".to_string())], 0, 13), "see label (https://x.example/a%20b) now");
    assert_eq!(selected_text(lt, &[], &[(4..9, "label".to_string())], 0, 13), lt);
    assert_eq!(selected_text(&text, &runs, &[], 1, 3), " 🫠");
}

#[test]
fn no_box_for_invisible_or_undrawable_characters() {
    let c = mc(&crate::dark());
    // A stray VS16 / ZWJ / BOM after an emoticon is dropped, the emoticon stays text.
    for s in ["live auf Staging :D\u{FE0F}", "live auf Staging :D\u{200D}", "live auf Staging :D\u{FEFF}"] {
        let (job, _, runs) = body_job(s, &c, "", 400.0, false);
        assert_eq!(job.text, "live auf Staging :D", "{s:?}");
        assert!(runs.is_empty());
    }
    // An emoji the picture set lacks and no system font can draw is hidden, not a box (no run, no placeholder).
    let (job, _, runs) = body_job("a \u{1FAEB} b", &c, "", 400.0, false);
    assert_eq!(job.text, "a  b");
    assert!(runs.is_empty());
    // Emoji in the set still become pictures.
    let (_, _, runs) = body_job("ok \u{1F44D}\u{FE0F}", &c, "", 400.0, false);
    assert_eq!(runs.len(), 1);
}

#[test]
fn only_web_links_are_opened() {
    for bad in ["javascript:alert(1)", "file:///C:/Windows/System32/cmd.exe", "ms-msdt:/id x", "\\\\host\\share", "ftp://x"] { assert_eq!(web_url(bad), None, "{bad}"); }
    assert_eq!(web_url("https://example.org").as_deref(), Some("https://example.org"));
    assert_eq!(web_url("www.example.com").as_deref(), Some("https://www.example.com"));
}

#[test]
fn mentions_are_found_case_insensitively() {
    let (job, _, _) = body_job("Hallo @alex muster und @alle", &mc(&crate::dark()), "Alex Muster", 400.0, false);
    assert!(job.sections.len() >= 3, "mention of me and @alle are separate coloured runs");
}

#[test]
fn image_boxes_follow_the_qml_limits() {
    let a = |w, h| Attachment { width: w, height: h, ..Default::default() };
    assert_eq!(image_box(&a(800, 510), 600.0, None), Vec2::new(320.0, 320.0 * 510.0 / 800.0)); // width 320, aspect kept
    let tall = image_box(&a(600, 1200), 600.0, None); // too tall: height 320, width follows
    assert_eq!((tall.y, tall.x), (320.0, 160.0));
    assert_eq!(image_box(&a(800, 510), 200.0, None).x, 200.0); // narrower than 320: available width
    assert_eq!(image_box(&a(0, 0), 600.0, None), Vec2::new(320.0, 320.0)); // unknown size: square until decoded
    assert_eq!(image_box(&a(0, 0), 600.0, Some([400, 200])), Vec2::new(320.0, 160.0)); // ... then the texture's
    assert_eq!(image_box(&a(800, 400), 600.0, Some([100, 200])), Vec2::new(160.0, 320.0)); // the decoded size wins over metadata
    assert_eq!(image_box(&a(512, 512), 600.0, Some([300, 600])), Vec2::new(160.0, 320.0)); // portrait never squashed
    assert_eq!(image_box(&a(0, 0), 600.0, Some([40, 30])), Vec2::new(40.0, 30.0)); // tiny: own size
    assert_eq!(image_box(&a(0, 0), 600.0, Some([8000, 4000])), Vec2::new(320.0, 160.0)); // huge: contained
}

#[test]
fn header_shows_the_full_date_only_for_older_days() {
    assert_eq!(header_time("2026-11-16", "16:48", "2026-11-16", "2026-11-15"), "16:48");
    assert_eq!(header_time("2026-11-15", "16:48", "2026-11-16", "2026-11-15"), "16:48");
    assert_eq!(header_time("2026-11-13", "16:48", "2026-11-16", "2026-11-15"), "13.11.2026, 16:48");
    assert_eq!(day_label("2026-11-13", "2026-11-16", "2026-11-15"), "13.11.2026");
    // The word depends on the global UI language (the i18n tests switch it concurrently): the logic is what is checked here.
    assert!(matches!(day_label("2026-11-15", "2026-11-16", "2026-11-15").as_str(), "Gestern" | "Yesterday"));
}

#[test]
fn arriving_messages_are_localised_once_and_keep_their_utc() {
    let mut v = vec![Message { id: "a".into(), date: "2026-11-13".into(), time: "15:48".into(), text: "x".into(), ..Default::default() },
        Message { id: "b".into(), date: "2026-11-13".into(), time: "now".into(), ..Default::default() }];
    prepare_arrived(&mut v);
    let local = to_local("2026-11-13", "15:48").unwrap();
    assert_eq!((v[0].date.clone(), v[0].time.clone()), local);
    assert_eq!(v[0].utc, Some(("2026-11-13".into(), "15:48".into())));
    assert_eq!(utc_date_time(&v[0]), ("2026-11-13", "15:48"));
    // Not a time: left as it is, not marked as localised.
    assert_eq!((v[1].time.as_str(), v[1].utc.is_none()), ("now", true));
    // A second arrival pass (e.g. the same message again) does not shift it twice.
    prepare_arrived(&mut v);
    assert_eq!((v[0].date.clone(), v[0].time.clone()), local);
    // A time-zone change converts again from the kept UTC values.
    relocalise(&mut v[0]);
    assert_eq!((v[0].date.clone(), v[0].time.clone()), local);
    assert_eq!(v[0].utc, Some(("2026-11-13".into(), "15:48".into())));
}

#[test]
fn content_revision_follows_what_the_list_draws() {
    let m = Message { id: "a".into(), text: "hallo".into(), ..Default::default() };
    let r = content_rev(&m);
    assert_ne!(r, 0);
    assert_eq!(r, content_rev(&m.clone()));
    let mut e = m.clone(); e.text = "hallo!".into(); assert_ne!(content_rev(&e), r);
    let mut e = m.clone(); e.edited = true; assert_ne!(content_rev(&e), r);
    let mut e = m.clone(); e.reactions.push(cloud::Reaction { name: "heart".into(), count: 1, mine: None, ..Default::default() }); assert_ne!(content_rev(&e), r);
    let mut e = m.clone(); e.html = "<b>hallo</b>".into(); assert_ne!(content_rev(&e), r);
    // The date / time are not part of it (the header is laid out at drawing time).
    let mut e = m.clone(); e.time = "10:00".into(); assert_eq!(content_rev(&e), r);
}

#[test]
fn utc_converts_to_local_time() {
    let (d, t) = to_local("2026-11-13", "15:48").unwrap();
    if cfg!(windows) { assert!(d == "2026-11-13" || d == "2026-11-14" || d == "2026-11-12"); } else { assert_eq!((d.as_str(), t.as_str()), ("2026-11-13", "15:48")); }
    assert_eq!(t.len(), 5);
    assert!(to_local("", "15:48").is_none() && to_local("2026-11-13", "now").is_none() && to_local("2026-13-01", "10:00").is_none());
}

#[test]
fn reaction_chips_never_show_raw_names() {
    assert_eq!(chip_emoji("thumbsup").as_deref(), Some("👍"));
    assert_eq!(chip_emoji("👍🏽").as_deref(), Some("👍🏽"));
    assert_eq!(chip_emoji("😀yellow").as_deref(), Some("😀"));
    assert_eq!(chip_emoji("yellow"), None);
    assert_eq!(chip_emoji(""), None);
}

#[test]
fn jump_finds_the_quoted_message_above_the_reply() {
    let m = |id: &str, t: &str| Message { id: id.into(), text: t.into(), ..Default::default() };
    let list = [m("a", "Build ist durch"), m("b", "ich schaff 50%"), m("c", "Build  ist durch"), m("r", "Immerhin etwas"), m("d", "Build ist durch")];
    assert_eq!(jump_target(&list, "r", "Build ist durch").as_deref(), Some("c")); // the newest one above the reply, spaces ignored
    assert_eq!(jump_target(&list, "r", "schaff").as_deref(), Some("b")); // partial match as the fallback
    assert_eq!(jump_target(&list, "r", "gibt es nicht"), None);
    assert_eq!(jump_target(&list, "r", ""), None);
}

#[test]
fn anchoring_follows_content_inserted_above() {
    assert_eq!(anchored_offset(100.0, 40.0, 340.0), Some(400.0)); // a 300 pt page was prepended: the view moves down with it
    assert_eq!(anchored_offset(100.0, 40.0, 40.3), None); // sub-pixel noise is ignored
    assert_eq!(anchored_offset(100.0, 80.0, 30.0), Some(50.0)); // content above shrank
    assert_eq!(anchored_offset(10.0, 80.0, 0.0), Some(0.0)); // never below the top
}

#[test]
fn older_pages_are_not_requested_while_the_chat_opens() {
    // The first frames after opening: offset 0 because the bottom isn't applied yet -> no request.
    assert!(!should_request_older(0, true, 0.0, 450.0, 1500.0, false, false, true, true));
    assert!(!should_request_older(2, true, 0.0, 450.0, 1500.0, false, false, true, true));
    assert!(!should_request_older(5, true, 0.0, 450.0, 1500.0, false, false, true, true)); // pinned to the bottom
    // Scrolled up near the top after settling: yes, once.
    assert!(should_request_older(5, false, 120.0, 450.0, 1500.0, false, false, true, true));
    assert!(!should_request_older(5, false, 120.0, 450.0, 1500.0, true, false, true, true)); // already loading
    assert!(!should_request_older(5, false, 120.0, 450.0, 1500.0, false, true, true, true)); // start reached
    assert!(!should_request_older(5, false, 900.0, 450.0, 1500.0, false, false, true, true)); // far from the top
    // A chat shorter than the view fills up.
    assert!(should_request_older(4, true, 0.0, 450.0, 0.0, false, false, true, true));
    assert!(!should_request_older(4, true, 0.0, 450.0, 0.0, false, false, false, true)); // nothing loaded at all
    // An older page just arrived and is still being anchored: no new request yet.
    assert!(!should_request_older(5, false, 120.0, 450.0, 1500.0, false, false, true, false));
}

fn rows(v: &[(&str, f32, f32)]) -> Vec<(String, f32, f32)> { v.iter().map(|(i, t, b)| (i.to_string(), *t, *b)).collect() }

#[test]
fn anchor_is_the_first_visible_row() {
    let r = rows(&[("a", 0.0, 50.0), ("b", 50.0, 120.0), ("c", 120.0, 200.0)]);
    assert_eq!(pick_anchor(&r, 0.0), Some(("a".into(), 0.0)));
    assert_eq!(pick_anchor(&r, 60.0), Some(("b".into(), 50.0))); // b is partly visible
    assert_eq!(pick_anchor(&r, 120.0), Some(("c".into(), 120.0))); // b ends exactly at the top: not visible
    assert_eq!(pick_anchor(&r, 500.0), None);
}

#[test]
fn rows_above_the_view_change_height_the_view_stays() {
    // Looking at "c" 20 pt below the top; a 300 pt page and a date row arrive above it.
    let prev = ("c".to_string(), 120.0);
    let now = rows(&[("p1", 0.0, 260.0), ("day", 260.0, 300.0), ("a", 300.0, 350.0), ("b", 350.0, 420.0), ("c", 420.0, 500.0)]);
    let f = scroll_fix(&now, Some(&prev), 100.0, 2000.0, 100.0, false, false, false);
    assert_eq!(f.set, Some(400.0)); // c's on-screen y: 120 - 100 = 20 before, 420 - 400 = 20 after
    assert!(f.redo, "the shifted frame must never be shown");
    assert_eq!(f.anchor, Some(("c".into(), 420.0)));
    // The redone pass: same layout, offset corrected -> nothing more to do.
    let g = scroll_fix(&now, f.anchor.as_ref(), 400.0, 2000.0, 400.0, false, true, false);
    assert_eq!((g.set, g.redo), (None, false));
    assert_eq!(g.anchor, Some(("b".into(), 350.0))); // b (350..420) is the first row still partly on screen at 400
}

#[test]
fn wheel_scrolling_while_rows_above_change() {
    // The wheel moved the offset 12 pt up this frame (112 -> 100) and the spinner row (48) appeared above everything.
    let prev = ("c".to_string(), 120.0);
    let now = rows(&[("spin", 0.0, 48.0), ("a", 48.0, 98.0), ("b", 98.0, 168.0), ("c", 168.0, 248.0)]);
    let f = scroll_fix(&now, Some(&prev), 100.0, 900.0, 100.0, false, false, false);
    assert_eq!(f.set, Some(148.0)); // the wheel's 12 pt stay, the 48 pt of the spinner are compensated
    // Rows below the anchor that change height need nothing.
    let below = rows(&[("a", 0.0, 50.0), ("b", 50.0, 120.0), ("c", 120.0, 400.0)]);
    let g = scroll_fix(&below, Some(&prev), 100.0, 900.0, 100.0, false, false, false);
    assert_eq!((g.set, g.redo), (None, false));
}

#[test]
fn pinned_list_redoes_frames_whose_height_changed() {
    let r = rows(&[("a", 0.0, 500.0), ("b", 500.0, 900.0)]);
    // Opened: laid out once invisibly, redone.
    assert!(scroll_fix(&r, None, 400.0, 400.0, 1.0e5, true, false, true).redo);
    // A link preview / "Gesehen von" / reload made the list 48 pt taller: the frame laid out at the old maximum is redone.
    let f = scroll_fix(&r, None, 352.0, 400.0, 352.0, true, false, false);
    assert!(f.redo && f.set.is_none());
    // Nothing changed.
    assert!(!scroll_fix(&r, None, 400.0, 400.0, 400.0, true, false, false).redo);
    // Explicitly set offsets (scrollbar, jump to a message) are never corrected.
    let prev = ("b".to_string(), 450.0);
    assert_eq!(scroll_fix(&r, Some(&prev), 100.0, 400.0, 100.0, false, true, false).set, None);
}

#[test]
fn seen_by_follows_marks_and_later_messages() {
    let m = |id: &str, email: &str, mine: bool| Message { id: id.into(), email: email.into(), author: email.split('@').next().unwrap_or("").into(), mine, ..Default::default() };
    let mark = |email: &str, activity: &str| cloud::ReadMark { person: format!("p-{email}"), email: email.into(), activity: activity.into(), time: String::new() };
    let keys = |v: Vec<(String, String, String)>| v.into_iter().map(|k| k.0).collect::<Vec<_>>();
    // My message (10:21), then Dario's (10:23); no acknowledge from him -> shown: he wrote the newest message.
    let chat = [m("a", "dario@x.example", false), m("b", "", true), m("c", "dario@x.example", false)];
    assert_eq!(seen_keys(&chat, &[]), [("dario@x.example".to_string(), String::new(), "dario".to_string())]);
    // An old mark (before the newest message) doesn't count, but being the newest message's author still does.
    assert_eq!(seen_keys(&chat, &[mark("Dario@x.example", "a")]).len(), 1);
    // The banner belongs to the conversation's newest message, not to mine: a mark on my (older) message no longer counts.
    let chat = [m("a", "x@x.example", false), m("b", "", true), m("c", "x@x.example", false), m("d", "z@x.example", false)];
    assert!(seen_keys(&chat, &[mark("y@x.example", "a")]).iter().all(|k| k.0 != "y@x.example"));
    assert!(seen_keys(&chat, &[mark("y@x.example", "b")]).iter().all(|k| k.0 != "y@x.example"));
    assert!(seen_keys(&chat, &[mark("y@x.example", "c")]).iter().all(|k| k.0 != "y@x.example"));
    assert!(seen_keys(&chat, &[mark("y@x.example", "d")]).iter().any(|k| k.0 == "y@x.example"));
    assert!(seen_keys(&chat, &[mark("y@x.example", "gone")]).iter().all(|k| k.0 != "y@x.example"));
    // x wrote earlier but hasn't acknowledged the newest message: not shown; z wrote it: shown.
    assert_eq!(keys(seen_keys(&chat, &[])), ["z@x.example"]);
    // A system notice after the newest message doesn't move the anchor.
    let with_notice = [m("a", "x@x.example", false), Message { id: "n".into(), notice: "call".into(), ..Default::default() }];
    assert_eq!(keys(seen_keys(&with_notice, &[mark("y@x.example", "a")])), ["y@x.example", "x@x.example"]);
    // Nobody twice; and no message of mine is needed at all.
    assert_eq!(seen_keys(&chat, &[mark("z@x.example", "d"), mark("Z@x.example", "d")]).len(), 1);
    assert_eq!(keys(seen_keys(&[m("a", "x@x.example", false)], &[mark("y@x.example", "a")])), ["y@x.example", "x@x.example"]);
    assert!(seen_keys(&[], &[mark("y@x.example", "a")]).is_empty());
}

#[test]
fn seen_by_newest_reader_first() {
    // Readers are sorted by read time, newest first.
    let m = |id: &str, email: &str, mine: bool| Message { id: id.into(), email: email.into(), mine, ..Default::default() };
    let mark = |email: &str, activity: &str, time: &str| cloud::ReadMark { person: format!("p-{email}"), email: email.into(), activity: activity.into(), time: time.into() };
    let chat = [m("a", "", true), m("b", "w@x.example", false)];
    let marks = [mark("p@x.example", "b", "2026-11-16T10:00:00.000Z"), mark("q@x.example", "b", "2026-11-16T10:05:00.000Z"), mark("r@x.example", "b", "2026-11-16T09:59:00.000Z")];
    let got: Vec<String> = seen_keys(&chat, &marks).into_iter().map(|k| k.0).collect();
    // The newest message's author without a mark of their own last (posting precedes every acknowledge of it).
    assert_eq!(got, ["q@x.example", "p@x.example", "r@x.example", "w@x.example"]);
    // The author's own newer acknowledge puts them in time order.
    let got: Vec<String> = seen_keys(&chat, &[marks[0].clone(), mark("w@x.example", "b", "2026-11-16T10:01:00.000Z")]).into_iter().map(|k| k.0).collect();
    assert_eq!(got, ["w@x.example", "p@x.example"]);
}

#[test]
fn seen_overflow_like_avatars_list() {
    // At most 10 avatars; more than 10 -> 9 avatars and "+(n - 9)".
    assert_eq!(seen_overflow(0), (0, 0));
    assert_eq!(seen_overflow(3), (3, 0));
    assert_eq!(seen_overflow(10), (10, 0));
    assert_eq!(seen_overflow(11), (9, 2));
    assert_eq!(seen_overflow(13), (9, 4));
}

#[test]
fn grouping_window_is_the_originals_60_s() {
    assert!(super::grouped_in_time(1_000, 61_000));
    assert!(!super::grouped_in_time(1_000, 61_001));
    assert!(super::grouped_in_time(61_000, 120_000), "measured from the previous message, not the first of the group");
}

#[test]
fn seen_row_is_placed_like_the_original() {
    // A 78.4 pt label and 2 avatars: the label starts 97.2 pt left of the centre.
    let (left, slot0) = super::seen_layout(0.0, 78.4, 2, 0.0);
    assert!((left + 97.2).abs() < 0.5, "{left}");
    assert!((slot0 + 1.0 - (left + 78.4 + 13.6)).abs() < 0.01);
    // The visible group stays centred 20 pt left of the centre whatever the count.
    for n in [1usize, 3, 9] { let (l, s) = super::seen_layout(100.0, 80.0, n, 0.0); assert!(((l + s + n as f32 * 32.0) / 2.0 - 80.0).abs() < 0.01); }
}

#[test]
fn minutes_and_reaction_names() {
    assert_eq!(minutes("16:09"), Some(16 * 60 + 9));
    assert_eq!(minutes("now"), None);
    assert_eq!(reaction_name("👍"), "thumbsup");
    assert_eq!(reaction_name("❤️"), "heart");
    assert_eq!(reaction_name("🙌"), "🙌"); // not a legacy name: the emoji itself
}
