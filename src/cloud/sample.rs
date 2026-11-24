use super::*;

/// Offline sample data so the app is usable (and checkable) without an account: spaces, order, presence and teams
/// of a typical German workspace.
pub fn sample() -> (Person, Vec<Space>, Vec<Team>, Vec<Person>, Vec<Meeting>) {
    let me = Person { id: "me".into(), name: "Alex Muster".into(), email: "you@example.com".into(), status: "active".into() };
    let sp = |id: &str, t: &str, d: bool, team: &str, last: &str, unread: bool, fav: bool| Space {
        id: id.into(), title: t.into(), direct: d, team: team.into(), last: last.into(), unread, favorite: fav, ..Default::default() };
    let mut s1 = sp("s1", "Webex-Bereich für Alex", false, "", "17:05", false, true);
    s1.participants = vec![me.clone()];
    s1.last_seen = format!("{}T09:35:00.000Z", crate::system::today()); // sample: "Neue Nachrichten" line before the 09:40 message
    let spaces = vec![
        s1,
        sp("s2", "Jonas Weber", true, "", "16:20", false, true),
        sp("s3", "Timo Steiner", true, "", "15:58", false, true),
        Space { public: true, ..sp("s4", "Onboarding Kickoff", false, "", "Mo", false, true) },
        sp("s5", "Nina Huber", true, "", "15:10", false, false),
        Space { meeting: true, ..sp("s6", "Development", false, "", "Mo", false, false) }, // sample: a space meeting in progress ("Beitreten")
        sp("s7", "Allgemein", false, "t3", "Mo", false, false),
        sp("s8", "Status Update", false, "", "Mo", false, false),
        sp("s9", "Mara Lind", true, "", "Mo", false, false),
        sp("s10", "Lina Roth", true, "", "Mo", false, false),
        sp("s11", "Projekt Alpha", false, "", "Mo", false, false),
        sp("s12", "Marketing", false, "", "Mo", false, false),
        // s13: an empty direct chat (the welcome item); s14: a closed direct chat whose other person is gone (no participant,
        // only its own name; the read-only notice).
        sp("s13", "Eva Sommer", true, "", "Mo", false, false),
        Space { closed: true, ..sp("s14", "Paul Graber", true, "", "Mo", false, false) },
        Space { announcement: true, ..sp("s15", "Ankündigungen", false, "", "Mo", false, false) }, // announcement space, not a moderator
    ];
    let teams = vec![Team { id: "t3".into(), name: "Product Team".into(), color: Some((0x2E, 0x8B, 0xE6)) }];
    let pp = |n: &str, e: &str, s: &str| Person { id: e.into(), name: n.into(), email: e.into(), status: s.into() };
    let people = vec![
        pp("Jonas Weber", "jonas@example.com", "inactive"), pp("Timo Steiner", "timo@example.com", "inactive"),
        pp("Nina Huber", "nina@example.com", "inactive"), pp("Mara Lind", "mara@example.com", "active"), pp("Lina Roth", "lina@example.com", "active"),
        pp("Eva Sommer", "eva@example.com", "active"),
    ];
    let meetings = vec![
        Meeting { title: "Weekly Sync".into(), start: "11:00".into(), link: "https://example.com/meet/weekly".into() },
        Meeting { title: "Design-Review".into(), start: "14:30".into(), link: "https://example.com/meet/design".into() },
    ];
    (me, spaces, teams, people, meetings)
}

pub fn sample_messages(room: &str) -> Vec<Message> {
    let m = |id: &str, a: &str, e: &str, t: &str, time: &str| Message { id: format!("{room}-{id}"), author: a.into(), email: e.into(),
        text: t.into(), time: time.into(), mine: e == "you@example.com", date: crate::system::today(), ..Default::default() };
    let me = ("Alex Muster", "you@example.com");
    let notice = |id: &str, text: &str, time: &str| Message { id: format!("{room}-{id}"), notice: text.into(), time: time.into(), date: crate::system::today(), ..Default::default() };
    let img = |key: &str, name: &str, w: u32, h: u32| Attachment { name: name.into(), mime: "image/png".into(), size: 0, key: key.into(), width: w, height: h, giphy: None };
    match room {
        // Everything the message list can show: yesterday/today separators, own and other messages, grouping, a reaction row,
        // a link and a mention, a file chip, an inline image with a caption, an edited message, one collapsed thread
        // (3 replies) and one open thread (2 replies).
        "s1" => {
            let mut first = m("1", me.0, me.1, "Hier sind die Architektur-Notizen zum Kranich-Portal.", "16:48");
            first.date = crate::system::days_ago(1);
            first.reactions = vec![Reaction { name: "thumbsup".into(), count: 1, mine: None, users: vec![Reactor { id: "mara@example.com".into(), ..Default::default() }] }];
            let mut second = m("2", me.0, me.1, "Sieht gut aus, ich schaue mir das morgen noch einmal an.", "16:49");
            second.date = crate::system::days_ago(1);
            let mut link = m("4", "Mara Lind", "mara@example.com", "Die Doku steht unter https://docs.example.com/kranich-portal/overview (und der Status auf www.example.com/status). @Alex Muster kannst du bitte kurz prüfen, ob die Beschreibung so stimmt und ob wir das Wording für die Release Notes noch einmal anpassen sollten?", "09:02");
            // Reactors (made-up people): six on the 👏 so its popover scrolls past five rows.
            let who = |l: &[(&str, &str)]| l.iter().map(|(id, t)| Reactor { id: id.to_string(), published: format!("{}T{t}:00.000Z", crate::system::today()), ..Default::default() }).collect::<Vec<_>>();
            link.reactions = vec![
                Reaction { name: "thumbsup".into(), count: 2, mine: Some("local".into()), users: who(&[("lina@example.com", "09:03"), ("me", "09:05")]) },
                Reaction { name: "heart".into(), count: 1, mine: None, users: who(&[("jonas@example.com", "09:04")]) },
                Reaction { name: "clap".into(), count: 6, mine: None, users: who(&[("mara@example.com", "09:03"), ("jonas@example.com", "09:04"), ("lina@example.com", "09:06"),
                    ("nina@example.com", "09:07"), ("timo@example.com", "09:12"), ("eva@example.com", "09:20")]) }];
            // Rich text (object.content): the link keeps %20, <br> is a hard break, a mention, formatting, a list and a quote.
            let url = "https://dev.azure.com/contoso/Contoso%20Apps/_git/Portal/pullrequest/4711";
            let mut rich = m("9", "Jonas Weber", "jonas@example.com", &format!("{url}\nBitte bis Freitag anschauen."), "09:08");
            rich.html = format!("<a href=\"{url}\">{url}</a><br/>Bitte bis Freitag anschauen.<br/><b>fett</b> <i>kursiv</i> <code>code</code> <spark-mention data-object-type=\"person\" data-object-id=\"x\">@Alex</spark-mention><ul><li>eins</li><li>zwei</li></ul><blockquote>zitiert</blockquote>");
            let mut head = m("12", "Jonas Weber", "jonas@example.com", "😀", "09:09");
            head.html = "<h1>\u{1F600}</h1>".into();
            let mut head2 = m("13", "Jonas Weber", "jonas@example.com", "Titel 😀", "09:09");
            head2.html = "<h2>Titel \u{1F600}</h2><h3>kleiner</h3>normal \u{1F600}".into();
            let mut old = m("11", "Jonas Weber", "jonas@example.com", "Aeltere Nachricht", "16:48");
            old.date = "2026-11-13".into();
            let mut cat = m("14", "Lina Roth", "lina@example.com", "", "09:11");
            cat.images = vec![Attachment { name: "Happy Cat".into(), mime: "image/gif".into(), size: 0, key: "sample-gif".into(), width: 480, height: 270,
                giphy: Some(Giphy { id: "xT9IgzoKnwFNmISR8I".into(), url: "https://media3.giphy.com/media/xT9IgzoKnwFNmISR8I/giphy.gif".into(), title: "Happy Cat".into() }) }];
            let mut card = m("8", "Jonas Weber", "jonas@example.com", "https://gitlab.com/example-group/filedrop", "09:10");
            card.previews = vec![LinkPreview { url: "https://gitlab.com/example-group/filedrop".into(), title: "example-group / filedrop · GitLab".into(), site: "GitLab".into(),
                description: "Send files between your paired devices over the local network. Desktop app and CLI for Linux and Windows.".into(), image_key: "sample-img-3".into() }];
            // Three files side by side (the gallery view's Flow): a long name (middle elision), a key file without extension and a
            // template (the "other" folder glyph), sizes in Byte / KB / MB wording.
            let mut file = m("5", "Lina Roth", "lina@example.com", "📎 Quartalsbericht-Q3-final-Version-2.pdf\n📎 deploy-key\n📎 Reisekosten_Vorlage_2026.xltx", "09:15");
            file.files = vec![Attachment { name: "Quartalsbericht-Q3-final-Version-2.pdf".into(), mime: "application/pdf".into(), size: 1_200_000, ..Default::default() },
                Attachment { name: "deploy-key".into(), size: 399, ..Default::default() },
                Attachment { name: "Reisekosten_Vorlage_2026.xltx".into(), size: 129_400, ..Default::default() }];
            file.date = crate::system::today();
            let mut shot = m("6", "Lina Roth", "lina@example.com", "Hier der Screenshot dazu", "09:16");
            shot.images = vec![img("sample-img-1", "screenshot.png", 800, 500)];
            // a shared video without thumbnail: the inline video card; the clip itself is made on this machine (media::video::synth)
            let mut clip = m("15", "Mara Lind", "mara@example.com", "📎 Demo-Clip.mp4", "09:18");
            clip.files = vec![Attachment { name: "Demo-Clip.mp4".into(), mime: "video/mp4".into(), size: 1_450_000, key: crate::media::video::synth::SAMPLE_KEY.into(), ..Default::default() }];
            let mut edited = m("7", me.0, me.1, "ok danke, mache ich", "09:20");
            edited.edited = true;
            let mut r1 = m("10", "Mara Lind", "mara@example.com", "Wer übernimmt das Review?", "09:30");
            r1.parent = String::new();
            let reply = |id: &str, a: &str, e: &str, t: &str, time: &str, parent: &str| { let mut x = m(id, a, e, t, time); x.parent = format!("{room}-{parent}"); x };
            vec![old, first, second, m("3", "Mara Lind", "mara@example.com", "Willkommen im Bereich.", "09:00"), link, rich, head, head2, cat, card, file, shot, clip, edited, r1,
                reply("11", "Lina Roth", "lina@example.com", "Ich kann das morgen früh machen.", "09:31", "10"),
                reply("12", "Alex Muster", me.1, "Super, danke dir!", "09:33", "10"),
                reply("13", "Mara Lind", "mara@example.com", "Perfekt, dann reviewt Lina.", "09:35", "10"),
                notice("19", "Mara Lind hat den Bereich verlassen.", "09:38"), m("20", "Lina Roth", "lina@example.com", "Standup um 10 Uhr?", "09:40"),
                reply("21", "Alex Muster", me.1, "Passt für mich.", "09:41", "20"),
                reply("22", "Mara Lind", "mara@example.com", "Bei mir auch.", "09:42", "20"),
                m("30", "Mara Lind", "mara@example.com", "Build ist durch", "09:44"),
                // Replies with a quote (object.content: <blockquote> with the author line); each one gets its own header.
                { let mut x = m("31", me.0, me.1, "Immerhin etwas", "09:45");
                    x.html = "<blockquote><span>Mara Lind</span><br/>Build ist durch</blockquote>Immerhin etwas".into(); x },
                { let mut x = m("32", me.0, me.1, "Bitte nicht um 10:30, da ist Daily", "09:46");
                    x.html = "<blockquote><span>Mara Lind</span><br/>Build ist durch</blockquote>Bitte nicht um 10:30, da ist Daily".into(); x }]
        }
        // Screenshot with caption, short own messages, a screenshot, "Gesehen von".
        "s2" => {
            let mut weekly = m("1", "Jonas Weber", "jonas@example.com", "Hmm", "15:22");
            weekly.images = vec![img("sample-img-2", "limit.png", 800, 190)];
            let mut shot = m("6", me.0, me.1, "", "16:09");
            shot.images = vec![img("sample-img-1", "screenshot.png", 800, 510)];
            vec![m("0", "Jonas Weber", "jonas@example.com", "perfekt", "15:12"), weekly,
                m("2", me.0, me.1, "oh", "15:23"), m("3", me.0, me.1, "hahaha", "15:23"),
                m("4", me.0, me.1, "Kaffee? Ich gehe sonst schon mal vor", "15:52"), notice("9", "Jonas Weber war nicht erreichbar.", "15:55"), shot,
                m("7", me.0, me.1, "lol", "16:09"), m("8", me.0, me.1, "noch nicht ganz fertig", "16:09")]
        }
        // A portrait screenshot taller than the message area, then several of my own messages in a row.
        "s3" => {
            let mut tall = m("1", "Timo Steiner", "timo@example.com", "Schau mal:", "15:50");
            tall.images = vec![img("sample-img-tall", "tall.png", 600, 1200)];
            vec![m("0", "Timo Steiner", "timo@example.com", "Hast du das neue Layout schon gesehen?", "15:49"), tall,
                m("2", me.0, me.1, "ja, sieht gut aus", "15:55"), m("3", me.0, me.1, "nur der Abstand oben ist noch zu gross", "15:55"),
                m("4", me.0, me.1, "xD", "15:56"), m("5", me.0, me.1, "ich schreibe das mal in den Bereich", "15:58"),
                m("6", me.0, me.1, "und dann sehen wir weiter", "15:58"),
                { let mut dup = m("8", "Timo Steiner", "timo@example.com", "😀\n😀", "16:02");
                    dup.reactions = vec![Reaction { name: "smiley".into(), count: 1, mine: None, ..Default::default() }, Reaction { name: "smiley".into(), count: 2, mine: None, ..Default::default() }];
                    dup },
                m("7", "Timo Steiner", "timo@example.com", "Haha stimmt 🫠 und noch 👍🏽 ❤️ 🇨🇭 🥹 1️⃣ 👨‍💻 🙌 😀", "16:01")]
        }
        "s13" | "s14" => vec![], // empty chats (welcome item / closed chat)
        _ => vec![m("1", "Mara Lind", "mara@example.com", "Willkommen im Bereich.", "09:00")],
    }
}
