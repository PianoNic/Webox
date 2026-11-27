//! What you can do to messages and spaces. Signed in, each action goes to the conversation
//! service (cloud::activities / cloud::space_ops); with sample data it changes the local copy.
use crate::*;
use cloud::Reaction;

/// What the message box is doing: a new message, a reply in a thread, or an edit.
#[derive(Clone, Default, PartialEq)]
pub(crate) enum Compose { #[default] New, Reply { root: String, to: String }, Edit { msg: String } }

/// Where a space's own draft waits while one of its messages is being edited (never saved: `save_drafts` skips these keys).
pub(crate) fn edit_stash_key(space: &str) -> String { format!("{space}\u{1}before-edit") }

pub(crate) fn markdown_default(local: Option<bool>, account: Option<bool>) -> bool { local.or(account).unwrap_or(true) }

/// Webox addition: a message of only 1-3 emoji (setting `big_emoji_send`, default on) is sent as the heading '# 😆' would be,
/// whatever the markdown mode. Returns the markdown source to send, or None when the text is not emoji-only.
pub(crate) fn big_emoji_source(text: &str) -> Option<String> {
    crate::ui::emoji_text::emoji_only(text).map(|e| format!("# {e}"))
}

#[cfg(test)]
mod markdown_tests {
    #[test]
    fn markdown_is_on_by_default_like_the_original() {
        assert!(super::markdown_default(None, None), "new install: the developer default markdown-enabled = true");
        assert!(!super::markdown_default(None, Some(false)), "the account's USER toggle");
        assert!(super::markdown_default(None, Some(true)));
        assert!(!super::markdown_default(Some(false), Some(true)), "switched off here");
        // '# 😀' with markdown on goes out as a heading (displayName the plain text, content the HTML).
        let r = crate::richtext::render("# 😀", super::markdown_default(None, None), &[]);
        assert_eq!((r.plain.as_str(), r.html.as_str()), ("😀", "<h1>😀</h1>"));
    }

    #[test]
    fn emoji_only_messages_are_sent_as_a_heading() {
        // The body that goes out (markdown forced on, whatever the composer's mode): the same as typing '# 😆'.
        let body = |s: &str| super::big_emoji_source(s).map(|src| { let r = crate::richtext::render(&src, true, &[]); (r.html, r.plain) });
        assert_eq!(body("😆"), Some(("<h1>😆</h1>".into(), "😆".into())));
        assert_eq!(body(" 👨‍👩‍👧 🇨🇭\n1️⃣ "), Some(("<h1>👨‍👩‍👧 🇨🇭 1️⃣</h1>".into(), "👨‍👩‍👧 🇨🇭 1️⃣".into())));
        assert_eq!(body("❤️❤️"), Some(("<h1>❤️❤️</h1>".into(), "❤️❤️".into())));
        assert_eq!(body("😆 hi"), None);
        assert_eq!(body("😆😆😆😆"), None);
        assert_eq!(body(":)"), None);
    }
}

impl App {
    pub(crate) fn send_text(&mut self, text: String, now: f64) { self.send_text_with(text, vec![], now) }

    /// A plugin's message for `space` (the `space.send` method): plain text as a new message, through the plugins' send hooks,
    /// without touching the composer. Dropped for unknown spaces and ones I cannot write in.
    pub(crate) fn post_to(&mut self, space: String, text: String) {
        if !self.spaces.iter().any(|s| s.id == space && !s.closed && !s.announcement) { return; }
        let text = text.trim();
        if text.is_empty() { return; }
        let Some(text) = self.plugins.on_send(text) else { return };
        match &self.client {
            Some(c) => c.req(Req::Post { space, text, markdown: false, mentions: vec![], kind: cloud::PostKind::New }),
            None => {
                let list = self.messages.entry(space.clone()).or_default();
                let id = format!("{space}-local{}", list.len());
                list.push(Message { id, author: self.me.name.clone(), email: self.me.email.clone(), text, time: "now".into(), date: system::today(), mine: true, ..Default::default() });
            }
        }
    }

    /// "Send single emojis large" (Webox addition, Messaging settings, default on).
    pub(crate) fn big_emoji_on(&self) -> bool { self.settings["big_emoji_send"].as_bool().unwrap_or(true) }

    /// Whether the composer's text is markdown: markdown mode, or the rich-text toolbar (its commands insert markdown markers).
    pub(crate) fn composer_is_markdown(&self) -> bool {
        self.markdown_on() || self.settings["composer_rich_toolbar"].as_bool().unwrap_or(false)
    }

    /// Markdown mode: the account's feature toggle `markdown-enabled` (a user toggle once the user switched it, otherwise the
    /// developer default `markdown-enabled = true`), so a new install starts with markdown on. With markdown off Webex composes
    /// rich text (toolbar formatting; "Enabling markdown will remove existing formatting."), it does not convert typed markdown.
    /// Webox's own `markdown_mode` setting wins once the user toggled it here (the user toggle is not written back to the service).
    pub(crate) fn markdown_on(&self) -> bool {
        markdown_default(self.settings["markdown_mode"].as_bool(), self.feature("markdown-enabled").and_then(|v| v.as_bool()))
    }

    /// Sends the composer text; `mentions` are the `@name`s chosen in the popup (signed in: they go out as `<spark-mention>`).
    /// Markdown is sent as `object.content` HTML (see richtext.rs).
    pub(crate) fn send_text_with(&mut self, text: String, mentions: Vec<crate::richtext::MentionRef>, now: f64) {
        crate::ui::theme::fx::sent(); // a design plugin's send burst (SEND_BURST), if on
        let Some(id) = self.selected.clone() else { return };
        let mut text = text.trim().to_string();
        if text.is_empty() { return; }
        if text.starts_with('/') {
            match self.plugins.command(&text) {
                Some(t) if t.is_empty() => return,
                Some(t) => text = t,
                // no plugin knows the command: it is just text starting with "/" (like "/fix" in a chat), sent as typed
                None => {}
            }
        }
        let Some(mut text) = self.plugins.on_send(&text) else { self.toast("A plugin stopped this message", now); return };
        let mode = std::mem::take(&mut self.compose);
        let mut markdown = self.composer_is_markdown();
        // Webox addition: emoji-only messages go out large (see `big_emoji_source`); never with mentions.
        let big = if mentions.is_empty() && self.big_emoji_on() { big_emoji_source(&text) } else { None };
        if let Some(src) = &big { text = src.clone(); markdown = true; }
        // Sample data: the local copy keeps the heading's HTML so it shows as sent.
        let local_html = || big.as_ref().map(|s| crate::richtext::render(s, true, &[]));
        match (&self.client, mode) {
            (Some(c), Compose::New) => c.req(Req::Post { space: id, text, markdown, mentions, kind: cloud::PostKind::New }),
            (Some(c), Compose::Reply { root, .. }) => c.req(Req::Post { space: id, text, markdown, mentions, kind: cloud::PostKind::Reply(root) }),
            (Some(c), Compose::Edit { msg }) => c.req(Req::Post { space: id, text, markdown, mentions, kind: cloud::PostKind::Edit(msg) }),
            (None, Compose::Edit { msg }) => {
                let r = local_html();
                if let Some(m) = self.messages.entry(id).or_default().iter_mut().find(|m| m.id == msg) {
                    match r { Some(r) => { m.text = r.plain; m.html = r.html; } None => m.text = text }
                    m.edited = true; m.rev = 0;
                }
            }
            (None, mode) => {
                let (text, html) = match local_html() { Some(r) => (r.plain, r.html), None => (text, String::new()) };
                let list = self.messages.entry(id.clone()).or_default();
                let parent = if let Compose::Reply { root, .. } = mode { root } else { String::new() };
                let mid = format!("{id}-local{}", list.len());
                let m = Message { id: mid, author: self.me.name.clone(), email: self.me.email.clone(), text, html, time: "now".into(), date: system::today(), mine: true, parent, ..Default::default() };
                // A reply goes after the last message of its thread, so the thread stays together.
                let at = if m.parent.is_empty() { list.len() } else { list.iter().rposition(|x| x.id == m.parent || x.parent == m.parent).map_or(list.len(), |i| i + 1) };
                list.insert(at, m);
            }
        }
    }

    /// Forwards a message's text to another space.
    pub(crate) fn forward(&mut self, to: &str, text: &str) {
        match &self.client {
            Some(c) => c.req(Req::Send(to.to_string(), text.to_string())),
            None => {
                let list = self.messages.entry(to.to_string()).or_default();
                let id = format!("{to}-local{}", list.len());
                list.push(Message { id, author: self.me.name.clone(), email: self.me.email.clone(), text: text.to_string(), time: "now".into(), date: system::today(), mine: true, ..Default::default() });
            }
        }
    }

    pub(crate) fn start_reply(&mut self, m: &Message) {
        let root = if m.parent.is_empty() { m.id.clone() } else { m.parent.clone() };
        self.compose = Compose::Reply { root, to: m.author.clone() };
    }

    /// Edit mode: the edited message gets a second input box over the conversation and the space's own draft is left alone,
    /// so the draft is put aside here and comes back when editing ends (`end_edit`). The editor gets the message's text as the
    /// data layer has it.
    pub(crate) fn start_edit(&mut self, m: &Message) {
        if let Some(id) = self.selected.clone() {
            if !matches!(self.compose, Compose::Edit { .. }) { let own = self.drafts.get(&id).cloned().unwrap_or_default(); self.drafts.insert(edit_stash_key(&id), own); }
            // The message's formatting comes back as the composer's markdown (lists, line breaks, bold, links ...).
            let md = if m.html.trim().is_empty() { m.text.clone() } else { crate::ui::messages::html_to_markdown(&m.html) };
            self.drafts.insert(id, md);
        }
        self.compose = Compose::Edit { msg: m.id.clone() };
    }

    /// Leaves edit mode (sent or cancelled) and gives the space its own draft back.
    pub(crate) fn end_edit(&mut self, space: &str) {
        let own = self.drafts.remove(&edit_stash_key(space)).unwrap_or_default();
        self.drafts.insert(space.to_string(), own);
        if matches!(self.compose, Compose::Edit { .. }) { self.compose = Compose::New; }
    }

    pub(crate) fn delete_message(&mut self, msg: &str) {
        let Some(id) = self.selected.clone() else { return };
        match &self.client {
            Some(c) => c.req(Req::Delete(id, msg.to_string())),
            None => if let Some(l) = self.messages.get_mut(&id) { l.retain(|m| m.id != msg && m.parent != msg) },
        }
    }

    /// Adds your reaction, or takes it back if you already reacted that way.
    pub(crate) fn toggle_reaction(&mut self, msg: &str, name: &str) {
        let Some(id) = self.selected.clone() else { return };
        let mine = self.messages.get(&id).and_then(|l| l.iter().find(|m| m.id == msg))
            .and_then(|m| m.reactions.iter().find(|r| r.name == name).map(|r| r.mine.clone()));
        match (&self.client, mine) {
            (Some(c), Some(Some(rid))) => c.req(Req::Unreact(id, rid)),
            (Some(c), _) => c.req(Req::React(id, msg.to_string(), name.to_string())),
            (None, mine) => {
                let Some(m) = self.messages.get_mut(&id).and_then(|l| l.iter_mut().find(|m| m.id == msg)) else { return };
                m.rev = 0; // edited in place: the list's layout cache hashes it again
                let me = cloud::Reactor { id: self.me.id.clone(), ..Default::default() };
                match (m.reactions.iter().position(|r| r.name == name), mine.flatten()) {
                    (Some(i), Some(_)) => {
                        let r = &mut m.reactions[i];
                        r.count -= 1; r.mine = None; r.users.retain(|u| u.id != me.id);
                        if r.count == 0 { m.reactions.remove(i); }
                    }
                    (Some(i), None) => { let r = &mut m.reactions[i]; r.count += 1; r.mine = Some("local".into()); r.users.push(me); }
                    (None, _) => m.reactions.push(Reaction { name: name.into(), count: 1, mine: Some("local".into()), users: vec![me] }),
                }
            }
        }
    }

    /// Read receipt for the newest message of the open space (verb "acknowledge"). Only while the window
    /// is in front (focused, not minimised): a chat that is merely selected behind other windows hasn't been read.
    /// The session sends it only when something is unseen and for the newest readable message (see `Session::ack`).
    pub(crate) fn acknowledge(&mut self, id: &str) {
        // Settings > Messaging "Use Read Receipts" off: no receipt on opening, only when sending a message.
        if self.settings["read_receipts"] == false { return; }
        if self.msgx.unread_hold.as_deref() == Some(id) { return; } // just marked unread: no receipt while it stays open
        if !self.window_active() { return }
        let (Some(c), Some(last)) = (&self.client, self.messages.get(id).and_then(|l| l.iter().rev().find(|m| m.notice.is_empty())).map(|m| m.id.clone())) else { return };
        if !last.is_empty() { c.req(Req::Ack(id.to_string(), last)); }
    }

    /// A feature toggle or client policy of the signed-in account (see cloud/features.rs): developer /
    /// entitlement / user toggles from the device registration, kept current by mercury `featureToggle_update`, else a device
    /// policy such as `ucf-giphy-rating`. `None` offline (sample) or when the account doesn't have the key.
    pub(crate) fn feature(&self, key: &str) -> Option<crate::cloud::features::FeatureValue> {
        if self.client.is_none() { return None }
        crate::cloud::features::feature(key)
    }

    /// When the window comes to the front, the open chat gets its read receipt then (it was held back
    /// while the window was in the background). Called every frame from `poll`; acts on the inactive → active change only.
    pub(crate) fn ack_on_activation(&mut self) {
        thread_local! { static WAS_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
        let active = self.window_active();
        if WAS_ACTIVE.with(|w| w.replace(active)) || !active { return }
        if let Some(id) = self.selected.clone() { self.acknowledge(&id); }
    }

    pub(crate) fn set_favorite(&mut self, id: &str, on: bool) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == id) { s.favorite = on; }
        if let Some(c) = &self.client { c.req(Req::Favorite(id.to_string(), on)); }
    }

    pub(crate) fn hide_space(&mut self, id: &str) {
        if let Some(c) = &self.client { c.req(Req::Hide(id.to_string())); }
        self.spaces.retain(|s| s.id != id);
        if self.selected.as_deref() == Some(id) { self.selected = None; } // nothing selected -> the create-space pane (no other space is opened or read)
    }

    pub(crate) fn leave_space(&mut self, id: &str) {
        if let Some(c) = &self.client { c.req(Req::Leave(id.to_string())); }
        self.spaces.retain(|s| s.id != id);
        if self.selected.as_deref() == Some(id) { self.selected = None; } // nothing selected -> the create-space pane (no other space is opened or read)
    }

    pub(crate) fn rename_space(&mut self, id: &str, title: &str) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == id) { s.title = title.to_string(); }
        if let Some(c) = &self.client { c.req(Req::Rename(id.to_string(), title.to_string())); }
    }

    pub(crate) fn add_person(&mut self, id: &str, p: &Person) {
        if let Some(s) = self.spaces.iter_mut().find(|s| s.id == id) {
            if !s.participants.iter().any(|x| x.id == p.id) { s.participants.push(p.clone()); }
        }
        if let Some(c) = &self.client { c.req(Req::AddPerson(id.to_string(), p.id.clone())); }
    }
}
