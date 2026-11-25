//! The message actions of the "···" toolbar button and the right-click menu, the quote banner of the composer and
//! "jump to a message" (in-space search, global search).
//!
//!   * Actions: quote, copy link, copy text, flag / unflag, mark unread, forward, edit, delete; icons text-blockquote_12,
//!     mark-as-unread_12, flag_12, copy_12, pin_12, edit_12, delete_12, forward-message_12, reply-bold.
//!   * Labels from the pack: "Quote message", "Flag message" / "Unflag message", "Mark as unread", "Forward message",
//!     "Edit message", "Delete message"; "Link copied"; "Remove quote". The "Replace existing quote" question is Webox's own.
//!   * Message link: "webexteams://im?space=" + id + "&message=" + id.
//!   * The menu's order and labels follow the Webex desktop menu (`message_items`); reminders are local (ui/msg_remind.rs),
//!     pinning and translation are shown disabled.
use super::menu;
use crate::*;
use egui::Id;
use crate::ui::widgets::tr;

/// The message being quoted in the composer of `space` (the quote banner above the input).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Quote { pub space: String, pub msg: String, pub author: String, pub email: String, pub html: String, pub text: String,
    /// The file names the quoted message shared (the quote names them: "Shared a file: …").
    pub files: Vec<String> }

/// State of the message actions (one App field).
#[derive(Default)]
pub(crate) struct MsgX {
    /// Message id -> flag url (needed to unflag; from `Resp::Flags` / `Resp::Flagged`).
    pub flag_urls: HashMap<String, String>,
    pub flags_requested: bool,
    /// The quote waiting in the composer.
    pub quote: Option<Quote>,
    /// A second quote asked for while one is waiting: the "Replace existing quote" question.
    pub replace: Option<Quote>,
    /// A space I marked unread while it is open: no read receipt for it until it is opened again.
    pub unread_hold: Option<String>,
    /// Messages scheduled with "Send later" (ui/schedule_send.rs).
    pub sched: Vec<super::schedule_send::Scheduled>,
    pub sched_expanded: bool,
    /// Link previews unfurled by this client (ui/unfurl.rs).
    pub unfurl: super::unfurl::Unfurl,
    /// "Remind me in" reminders, soonest first (ui/msg_remind.rs).
    pub reminders: Vec<super::msg_remind::Reminder>,
}

/// `webexteams://im?space=<space>&message=<message>` (the message link).
pub(crate) fn message_link(space: &str, msg: &str) -> String { format!("webexteams://im?space={space}&message={msg}") }

/// The quote's preview line in the composer banner: the first non-empty line of the quoted text.
pub(crate) fn quote_preview(text: &str) -> String { text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string() }

/// The message's own entries, in the order of the Webex desktop menu: Copy message, Edit message (mine), Start a
/// thread, Add reaction ›, Forward message, Remind me in ›, Pin message, Quote message, Translate
/// message, Flag message, Mark as unread (not mine), Delete message (mine). Pinning and translation have no
/// service behind them in Webox yet: they are shown, disabled.
pub(crate) fn message_items(m: &Message, flagged: bool) -> Vec<menu::Entry> {
    let mut v = vec![menu::Item::new("copy-message", tr("Copy message")).icon("copy-regular").entry()];
    if m.mine { v.push(menu::Item::new("edit", tr("Edit message")).icon("edit-regular").shortcut(tr("Shift+Up arrow")).entry()); }
    // A reply already is in a thread: "Reply to thread"; a root message starts one.
    v.push(menu::Item::new("reply", if m.parent.is_empty() { tr("Start a thread") } else { tr("Reply to thread") }).icon("reply-regular").entry());
    v.push(menu::Item::new("react", tr("Add reaction")).icon("reactions-regular").submenu().entry());
    v.push(menu::Item::new("forward", tr("Forward message")).icon("forward-message-regular").entry());
    v.push(menu::Item::new("remind", tr("Remind me in")).icon("alarm-regular").submenu().entry());
    v.push(menu::Item::new("pin", tr("Pin message")).icon("pin-regular").disabled().entry());
    v.push(menu::Item::new("quote", tr("Quote message")).icon("text-blockquote_12").entry());
    v.push(menu::Item::new("translate", tr("Translate message")).icon("translate-regular").disabled().entry());
    v.push(menu::Item::new("flag", if flagged { tr("Unflag message") } else { tr("Flag message") }).icon(if flagged { "flag-filled" } else { "flag-regular" }).entry());
    // My own messages can't make the space unread for me.
    if !m.mine { v.push(menu::Item::new("unread", tr("Mark as unread")).icon("mark-as-unread-regular").entry()); }
    if m.mine { v.push(menu::Item::new("delete", tr("Delete message")).icon("delete-regular").danger().entry()); }
    v
}

impl App {
    /// The entries of the "···" menu and the right-click menu (after the right-click menu's own text entries).
    pub(crate) fn msg_menu_entries(&self, id: &str, m: &Message) -> Vec<menu::Entry> {
        let mut v = message_items(m, self.flags.contains(&(id.to_string(), m.id.clone())));
        // With a reminder set, the official client's pair: "Edit reminder ›" and "Remove reminder".
        if self.msgx.reminders.iter().any(|r| r.msg == m.id) {
            if let Some(i) = v.iter().position(|e| matches!(e, menu::Entry::Item(it) if it.id == "remind")) {
                if let menu::Entry::Item(it) = &mut v[i] { it.label = tr("Edit reminder").into(); }
                v.insert(i + 1, menu::Item::new("remind-remove", tr("Remove reminder")).icon("alarm-regular").entry());
            }
        }
        v
    }

    /// Runs a picked entry of `msg_menu_entries` (and the right-click menu's link entries).
    pub(crate) fn msg_menu_do(&mut self, ctx: &egui::Context, id: &str, m: &Message, action: &str, link: Option<&str>) {
        let now = ctx.input(|i| i.time);
        match action {
            "reply" => self.start_reply(m),
            "forward" => self.forward_text = Some(m.text.clone()),
            "edit" => self.start_edit(m),
            "quote" => self.quote_message(ctx, id, m),
            "copy-msg-link" => { ctx.copy_text(message_link(id, &m.id)); self.toast(tr("Link copied"), now); }
            "copy-message" => ctx.copy_text(m.text.clone()),
            "flag" => self.toggle_flag(id, &m.id, now),
            "unread" => self.mark_unread(id, &m.id),
            "remind-remove" => self.remind_remove(&m.id),
            "delete" => ctx.data_mut(|d| d.insert_temp(Id::new("msg-delete-confirm"), m.id.clone())),
            "open-link" => if let Some(l) = link { if let Some(u) = web_url(l) { system::open(&u); } },
            "copy-link" => if let Some(l) = link { ctx.copy_text(l.to_string()); },
            _ => {}
        }
    }

    /// Flag / unflag. Signed in it goes to the flags service and the state follows its answer (`on_flagged`); offline it's local.
    pub(crate) fn toggle_flag(&mut self, space: &str, msg: &str, now: f64) {
        let key = (space.to_string(), msg.to_string());
        let on = !self.flags.contains(&key);
        match &self.client {
            Some(c) => {
                let flag_url = self.msgx.flag_urls.get(msg).cloned().unwrap_or_default();
                if !on && flag_url.is_empty() { self.toast(tr("Failed to change the flagged state of the item."), now); return; }
                c.req(Req::Flag { space: space.into(), msg: msg.into(), on, flag_url });
            }
            None => { if on { self.flags.insert(key); } else { self.flags.remove(&key); } }
        }
    }

    pub(crate) fn on_flags(&mut self, v: Vec<(String, String, String)>) {
        self.flags.clear();
        self.msgx.flag_urls.clear();
        for (space, msg, url) in v {
            // A flag without its conversation: find the space among the loaded messages (or keep it unplaced).
            let space = if space.is_empty() { self.messages.iter().find(|(_, l)| l.iter().any(|m| m.id == msg)).map(|(k, _)| k.clone()).unwrap_or_default() } else { space };
            self.msgx.flag_urls.insert(msg.clone(), url);
            self.flags.insert((space, msg));
        }
    }

    pub(crate) fn on_flagged(&mut self, space: String, msg: String, url: Option<String>) {
        match url {
            Some(u) => { self.msgx.flag_urls.insert(msg.clone(), u); self.flags.insert((space, msg)); }
            None => { self.msgx.flag_urls.remove(&msg); self.flags.retain(|f| f.1 != msg); }
        }
    }

    /// tr("Mark as unread"): the space shows as unread again and gets no read receipt until it is opened again.
    pub(crate) fn mark_unread(&mut self, space: &str, msg: &str) {
        if let Some(c) = &self.client { c.req(Req::MarkUnread { space: space.into(), msg: msg.into() }); }
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == space) { s.unread = true; }
        self.msgx.unread_hold = Some(space.to_string());
    }

    /// tr("Quote message"): the message goes into the composer's quote banner (asks first when another quote is waiting).
    pub(crate) fn quote_message(&mut self, ctx: &egui::Context, space: &str, m: &Message) {
        let q = Quote { space: space.into(), msg: m.id.clone(), author: if m.mine { self.me.name.clone() } else { m.author.clone() }, email: m.email.clone(), html: m.html.clone(), text: m.text.clone(), files: crate::cloud::shared_names(m) };
        match &self.msgx.quote {
            Some(old) if old.space == space && old.msg != m.id => self.msgx.replace = Some(q),
            _ => self.msgx.quote = Some(q),
        }
        if matches!(self.compose, Compose::Edit { .. }) { self.end_edit(space); } // leaves edit mode, the own draft returns
        ctx.memory_mut(|mm| mm.request_focus(Id::new("composer-edit")));
    }

    /// The quote waiting for `space`, taken out (the composer sends it with the next message).
    pub(crate) fn quote_take(&mut self, space: &str) -> Option<Quote> {
        if self.msgx.quote.as_ref().is_some_and(|q| q.space == space) { self.msgx.quote.take() } else { None }
    }
    pub(crate) fn quote_waiting(&self, space: &str) -> bool { self.msgx.quote.as_ref().is_some_and(|q| q.space == space) }

    /// Sends `text` with the quote `q` (as a new message, or in the thread the composer is replying to).
    pub(crate) fn send_quoted(&mut self, space: &str, text: String, mentions: Vec<crate::richtext::MentionRef>, q: Quote) {
        let markdown = self.composer_is_markdown();
        let parent = match std::mem::take(&mut self.compose) { Compose::Reply { root, .. } => Some(root), _ => None };
        match &self.client {
            Some(c) => c.req(Req::PostQuote { space: space.into(), text, markdown, mentions, quoted_html: q.html, quoted_text: q.text, quoted_files: q.files, author: q.author, author_email: q.email, parent }),
            None => {
                // Offline: the message as the reader shows quotes (author line + quoted text in a blockquote).
                let r = crate::richtext::render(&text, markdown, &[]);
                let html = crate::cloud::quote_preview_html(&q.author, &q.html, &q.text, &q.files, &r, &text);
                let list = self.messages.entry(space.to_string()).or_default();
                let mid = format!("{space}-local{}", list.len());
                let parent = parent.unwrap_or_default();
                let m = Message { id: mid, author: self.me.name.clone(), email: self.me.email.clone(), text, html, time: "now".into(), date: system::today(), mine: true, parent, ..Default::default() };
                let at = if m.parent.is_empty() { list.len() } else { list.iter().rposition(|x| x.id == m.parent || x.parent == m.parent).map_or(list.len(), |i| i + 1) };
                list.insert(at, m);
            }
        }
    }

    /// The quote banner (wx-quoteWidget-background, a bar in wx-quoteWidgetBar-background, the remove button) floating above the
    /// composer line, above a staged GIF if there is one.
    pub(crate) fn quote_banner(&mut self, ui: &mut egui::Ui, t: &Th, crect: Rect, space: &str) {
        let Some(q) = self.msgx.quote.clone().filter(|q| q.space == space) else { return };
        let ctx = ui.ctx().clone();
        let light = menu::light(t);
        let (bg, bar_c) = if light { (hex(0xF7F7F7), hex(0x707070)) } else { (hex(0x292929), hex(0xB2B2B2)) }; // token values Dark / Default
        let lift = if self.gif_staged(space) { 72.0 } else { 0.0 } + super::attach::upload_strips_height(space); // above running uploads
        let h = 52.0;
        let strip = Rect::from_min_max(pos2(crect.left(), crect.top() - lift - h - 4.0), pos2(crect.right(), crect.top() - lift));
        let mut remove = false;
        egui::Area::new(Id::new("quote-banner")).fixed_pos(strip.min).order(egui::Order::Middle).interactable(true).show(&ctx, |ui| {
            ui.allocate_exact_size(strip.size(), Sense::click());
            let p = ui.painter().clone();
            p.rect_filled(strip, 0.0, t.composer);
            let card = Rect::from_min_max(pos2(strip.left() + 20.0, strip.top() + 4.0), pos2(strip.right() - 20.0, strip.bottom() - 2.0));
            p.rect_filled(card, 8.0, bg);
            p.rect_filled(Rect::from_min_size(pos2(card.left() + 10.0, card.top() + 8.0), Vec2::new(3.0, card.height() - 16.0)), 1.5, bar_c);
            let tx = card.left() + 22.0;
            let maxw = card.width() - 22.0 - 40.0;
            super::chrome::fit_text(&p, tx, card.top() + 15.0, &q.author, semibold(13.0), t.text1, maxw);
            super::chrome::fit_text(&p, tx, card.top() + 34.0, &Some(quote_preview(&q.text)).filter(|l| !l.is_empty() && !l.starts_with('📎')).or_else(|| crate::cloud::Shared::of(&q.files).map(|s| s.label())).unwrap_or_default(), FontId::proportional(14.0), t.text2, maxw);
            let xr = Rect::from_center_size(pos2(card.right() - 20.0, card.center().y), Vec2::splat(24.0));
            let xresp = ui.interact(xr, Id::new("quote-banner-x"), Sense::click());
            if xresp.hovered() { p.circle_filled(xr.center(), 12.0, t.hover); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(12.0)), t.text1);
            if xresp.on_hover_text(tr("Remove quote")).clicked() { remove = true; }
        });
        if remove { self.msgx.quote = None; }
    }

    /// "Replace existing quote" when a second message is quoted.
    pub(crate) fn quote_replace_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if self.msgx.replace.is_none() { return; }
        let res = menu::webex_dialog(ctx, t, "quote-replace", tr("Replace existing quote"), tr("Your message draft already contains a quote.\nDo you want to replace it with this one?"), None,
            &[menu::Button::secondary("cancel", tr("Cancel")), menu::Button::primary("replace", tr("Replace"))]);
        match res.as_deref() {
            Some("replace") => { self.msgx.quote = self.msgx.replace.take(); }
            Some(_) => { self.msgx.replace = None; }
            None => {}
        }
    }

    /// A click on a person mention (`<person id>\u{1}<shown name>`): the person's contact card (by id, else by the shown name).
    pub(crate) fn mention_open(&mut self, ctx: &egui::Context, target: &str) {
        let (pid, name) = target.split_once('\u{1}').unwrap_or((target, ""));
        let all: Vec<Person> = self.people.iter().cloned().chain(self.spaces.iter().flat_map(|s| s.participants.iter().cloned())).collect();
        let p = all.iter().find(|p| p.id == pid || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(pid)))
            .or_else(|| all.iter().find(|p| !name.is_empty() && p.name.eq_ignore_ascii_case(name)))
            .cloned().unwrap_or(Person { id: pid.to_string(), name: name.to_string(), ..Default::default() });
        let p = self.live_person(&p);
        Self::contact_card_open(ctx, &p);
    }

    /// Opens `space` and scrolls its message list to `msg` (which flashes), opening a collapsed thread on the way.
    pub(crate) fn goto_message(&mut self, ctx: &egui::Context, space: &str, msg: &str) {
        if self.selected.as_deref() != Some(space) { self.open_space(space.to_string()); }
        self.conv_tab = ConvTab::Messages;
        if let Some(root) = self.messages.get(space).and_then(|l| l.iter().find(|m| m.id == msg)).map(|m| m.parent.clone()).filter(|p| !p.is_empty()) {
            self.open_threads.insert(root);
        }
        ctx.data_mut(|d| d.insert_temp(Id::new("msg-goto"), (space.to_string(), msg.to_string(), 0u32)));
    }

    /// Once per frame from the message list: load my flags once signed in, place flags whose space wasn't known, end the
    /// "marked unread" hold when another space is shown, and send scheduled messages that are due.
    pub(crate) fn msgx_tick(&mut self, ctx: &egui::Context) {
        if let Some(c) = &self.client {
            if !self.msgx.flags_requested && !self.spaces.is_empty() { self.msgx.flags_requested = true; c.req(Req::Flags); }
        }
        let unplaced: Vec<String> = self.flags.iter().filter(|f| f.0.is_empty()).map(|f| f.1.clone()).collect();
        for msg in unplaced {
            if let Some(space) = self.messages.iter().find(|(_, l)| l.iter().any(|m| m.id == msg)).map(|(k, _)| k.clone()) {
                self.flags.remove(&(String::new(), msg.clone()));
                self.flags.insert((space, msg));
            }
        }
        if self.msgx.unread_hold.is_some() && self.msgx.unread_hold != self.selected { self.msgx.unread_hold = None; }
        self.sched_tick(ctx);
        self.reminders_tick(ctx);
        // Screenshot checks only (--sample --screenshot): a waiting quote, scheduled messages, the "Send later" menu / dialog.
        // WEBOX_SHOT_EDITED=<n>:<text>[;<n>:<text>...]: message <n> is edited to <text>, one edit after the other, and
        // WEBOX_SHOT_KEPT=<n>[,<n>...]: those messages are deleted, through the plugins' message.edited / message.deleted path
        // (a logging plugin keeps them). WEBOX_SHOT_EDIT_DIALOG=<n> clicks the "(edited)" of message <n>.
        if self.shot.is_some() && self.frames == 3 {
            if let Some(sel) = self.selected.clone() {
                let name = self.spaces.iter().find(|s| s.id == sel).map(|s| s.title.clone()).unwrap_or_default();
                let step = |me: &mut Self, f: &dyn Fn(&Message) -> Message| if let Some(old) = me.messages.get(&sel).cloned() {
                    let mut new: Vec<Message> = old.iter().map(f).collect();
                    me.plugins.list_arrived(crate::extras::plugins::kept::Where { space: &sel, name: &name, ..Default::default() }, Some(&old), &mut new);
                    crate::ui::messages::prepare_arrived(&mut new);
                    me.messages.insert(sel.clone(), new);
                };
                for e in std::env::var("WEBOX_SHOT_EDITED").unwrap_or_default().split(';').filter_map(|e| e.split_once(':')) {
                    let (mid, text) = (format!("{sel}-{}", e.0), e.1.to_string());
                    step(self, &|m: &Message| if m.id == mid { Message { text: text.clone(), html: String::new(), edited: true, rev: 0, ..m.clone() } } else { m.clone() });
                }
                for v in std::env::var("WEBOX_SHOT_KEPT").unwrap_or_default().split(',').filter(|v| !v.is_empty()) {
                    let mid = format!("{sel}-{v}");
                    step(self, &|m: &Message| if m.id == mid { Message { id: m.id.clone(), notice: crate::trq!("%1 deleted the message.", m.author), ..Default::default() } } else { m.clone() });
                }
            }
        }
        if self.shot.is_some() && self.frames == 8 {
            if let (Some(sel), Ok(v)) = (self.selected.clone(), std::env::var("WEBOX_SHOT_EDIT_DIALOG")) {
                if let Some(m) = self.messages.get(&sel).and_then(|l| l.iter().find(|m| m.id == format!("{sel}-{v}"))) {
                    let p = json!({ "space": sel, "id": m.id, "text": m.text, "mine": m.mine, "published_ms": m.published_ms });
                    self.plugins.emit("message.edit_tag_clicked", &p.to_string());
                }
            }
        }
        if self.shot.is_some() && self.frames == 6 {
            if let Some(sel) = self.selected.clone() {
                if let Ok(v) = std::env::var("WEBOX_SHOT_QUOTE") {
                    if let Some(m) = self.messages.get(&sel).and_then(|l| l.iter().find(|m| m.id == format!("{sel}-{v}"))).cloned() { self.quote_message(ctx, &sel, &m); }
                }
                if std::env::var("WEBOX_SHOT_SCHED").is_ok() {
                    let due = super::schedule_send::tomorrow_nine(super::schedule_send::now_ms());
                    for (i, t) in ["Don't forget: daily at 9", "The report is in the folder"].iter().enumerate() {
                        self.msgx.sched.push(super::schedule_send::Scheduled { id: format!("shot{i}"), space: sel.clone(), text: t.to_string(), markdown: false, mentions: vec![], due_ms: due + i as i64 * 3_600_000 });
                    }
                    self.msgx.sched_expanded = std::env::var("WEBOX_SHOT_SCHED").as_deref() == Ok("open");
                }
                if std::env::var("WEBOX_SHOT_SCHED_MENU").is_ok() { self.drafts.insert(sel.clone(), "See you tomorrow".into()); super::menu::menu_open(ctx, "sched-menu"); }
                if std::env::var("WEBOX_SHOT_SCHED_CUSTOM").is_ok() { ctx.data_mut(|d| d.insert_temp(Id::new("sched-shot-custom"), true)); }
            }
        }
        if ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("sched-shot-custom"))).is_some() { self.sched_custom_open_pub(ctx); }
    }
}

/// The URL to open for a clicked link: http(s) as is, "www." completed to https, anything else nothing (as messages.rs).
fn web_url(u: &str) -> Option<String> {
    let u = if u.starts_with("www.") { format!("https://{u}") } else { u.to_string() };
    (u.starts_with("https://") || u.starts_with("http://")).then_some(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_links_and_previews() {
        assert_eq!(message_link("c-1", "a-2"), "webexteams://im?space=c-1&message=a-2");
        assert_eq!(quote_preview("\n  first line \nsecond"), "first line");
        assert_eq!(quote_preview(""), "");
    }
}
