//! Settings > Customize the interface: which UI elements are shown and, for the rail and the composer toolbar, in which order.
//!
//! Each area is a [`Group`] stored under its own settings key, e.g.
//! `"ui_rail": { "hidden": ["calling"], "order": ["contacts", "messaging", ...] }`. A missing key is today's look, so
//! "Reset" just removes it. Hiding is presentation only: shortcuts keep working, hidden rail entries move into the
//! rail's "…" menu, and the rail's settings gear and "…" button are never hidden together (Settings stays reachable).
//! The layout code calls [`shown`] / [`visible`] every frame: map lookups and a scan of a short array, no allocation.
use serde_json::{json, Value};

/// One element: its id in the settings JSON, the English label (translated with `tr` when drawn) and icon of the official app, and whether it keeps its place
/// (the rail's bottom entries) instead of taking part in the order.
pub(crate) struct El { pub id: &'static str, pub label: &'static str, pub icon: &'static str, pub fixed: bool }
const fn el(id: &'static str, label: &'static str, icon: &'static str) -> El { El { id, label, icon, fixed: false } }
const fn pinned(id: &'static str, label: &'static str, icon: &'static str) -> El { El { id, label, icon, fixed: true } }

/// One customisable area.
pub(crate) struct Group { pub key: &'static str, pub title: &'static str, pub desc: &'static str, pub els: &'static [El], pub orderable: bool }

/// Most elements a group may have (the order is kept in a fixed array).
pub(crate) const MAX: usize = 16;

pub(crate) const RAIL: Group = Group { key: "ui_rail", title: "App sidebar", orderable: true,
    desc: "Show or hide tabs and order them with the arrows. Hidden tabs are in the More menu; their shortcuts keep working.",
    els: &[el("messaging", "Messaging", "chat-filled"), el("meetings", "Meetings", "calendar-month-filled"), el("calling", "Calling", "handset-filled"),
        el("separator", "Divider", "horizontal-line-regular"), el("whiteboards", "Whiteboards", "whiteboard-content-filled"), el("contacts", "Contacts", "contacts-filled"),
        el("teams", "Teams", "webex-teams-filled"), el("more", "More", "more-regular"),
        pinned("help", "Help", "help-circle-filled"), pinned("settings", "Settings", "settings-filled")] };
pub(crate) const MORE: Group = Group { key: "ui_more", title: "More menu", orderable: false,
    desc: "Entries in the More menu of the app sidebar.",
    els: &[el("voicemail", "Voicemail", "voicemail-regular"), el("plugins", "Plugins & Themes", "apps-regular"), el("about", "About Webox", "info-circle-regular"),
        el("labels", "Show labels (Alt+Shift+Q)", "application-panel-regular")] };
pub(crate) const COMPOSER: Group = Group { key: "ui_composer", title: "Message box toolbar", orderable: true,
    desc: "Buttons under the message box. The shortcuts (Ctrl+O, Alt+Shift+M, Ctrl+Shift+Space) work without the buttons too.",
    els: &[el("attach", "Attach a file", "attachment_16_em"), el("screenshot", "Screen Capture", "screenshot_16_em"), el("markdown", "Markdown", "markdown_16_em"),
        el("emoji", "Emoji and symbols", "emoticons_16_em"), el("gif", "Add GIFs", "gif_16"), el("mention", "@Mentions", "mention_16_em"),
        el("ai", "AI Assistant", "sparkle-bold")] };
pub(crate) const HEADER: Group = Group { key: "ui_header", title: "Top bar", orderable: false,
    desc: "Buttons at the top of the window. Search also opens with Ctrl+F.",
    els: &[el("search", "Search, meet, and call", "search-regular"), el("plus", "Start a conversation", "plus-regular"),
        el("device", "Connect to a device", "device-connection-regular"), el("ai", "AI Assistant", "sparkle-regular")] };
pub(crate) const SPACE_TABS: Group = Group { key: "ui_space_tabs", title: "Space tabs", orderable: false,
    desc: "The tabs next to Messages at the top of a space.",
    els: &[el("people", "People / Profile", "people-regular"), el("content", "Content", "document-regular"), el("meetings", "Meetings", "meetings-regular"), el("apps", "+Apps", "apps-regular")] };
pub(crate) const MSG_BAR: Group = Group { key: "ui_msg_bar", title: "Message actions", orderable: false,
    desc: "The bar above a message when you point at it. More actions (···) always stays and holds the other actions.",
    els: &[el("quick", "Quick reactions (thumbs up, grinning face, raised hands)", "reactions-regular"), el("react", "Add reaction", "reactions_12"), el("edit", "Edit", "edit_12"),
        el("reply", "Reply to thread", "reply_12"), el("forward", "Forward", "forward-message_12")] };
pub(crate) const LIST: Group = Group { key: "ui_list", title: "Space list", orderable: false, desc: "",
    els: &[el("recommended", "Recommended messages", "chat-regular")] };

/// All groups in page order.
pub(crate) const GROUPS: [&Group; 7] = [&RAIL, &MORE, &COMPOSER, &HEADER, &SPACE_TABS, &MSG_BAR, &LIST];

fn hidden_raw(s: &Value, g: &Group, id: &str) -> bool { s[g.key]["hidden"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(id))) }

/// Whether element `id` of `g` is drawn. Should both the rail's gear and its "…" button be hidden (a hand-edited file), the gear stays.
pub(crate) fn shown(s: &Value, g: &Group, id: &str) -> bool {
    if !hidden_raw(s, g, id) { return true; }
    g.key == RAIL.key && id == "settings" && hidden_raw(s, g, "more")
}

/// Whether `id` may be hidden now: not the gear while the "…" button is hidden, nor the other way round.
pub(crate) fn can_hide(s: &Value, g: &Group, id: &str) -> bool {
    if g.key != RAIL.key { return true; }
    match id { "settings" => !hidden_raw(s, g, "more"), "more" => !hidden_raw(s, g, "settings"), _ => true }
}

/// The group's object in `s`, created (or replaced, if the file holds something else there) as needed.
fn group_mut<'a>(s: &'a mut Value, g: &Group) -> &'a mut Value {
    if !s.is_object() { *s = json!({}); }
    let v = &mut s[g.key];
    if !v.is_object() { *v = json!({}); }
    v
}

/// Shows or hides `id`. False (and nothing changes) when hiding it would make Settings unreachable.
pub(crate) fn set_shown(s: &mut Value, g: &Group, id: &str, show: bool) -> bool {
    if !show && !can_hide(s, g, id) { return false; }
    let h = &mut group_mut(s, g)["hidden"];
    if !h.is_array() { *h = json!([]); }
    let a = h.as_array_mut().expect("array");
    a.retain(|v| v.as_str() != Some(id));
    if !show { a.push(json!(id)); }
    true
}

/// The orderable elements of `g` in the user's order (indices into `g.els`): stored ids first (unknown ones and repeats
/// skipped), then any element the stored order doesn't name, in default order.
pub(crate) fn order(s: &Value, g: &Group) -> ([u8; MAX], usize) {
    let (mut out, mut n, mut used) = ([0u8; MAX], 0usize, 0u32);
    let mut put = |i: usize| if used & (1 << i) == 0 && !g.els[i].fixed { used |= 1 << i; out[n] = i as u8; n += 1; };
    if let Some(a) = s[g.key]["order"].as_array() {
        for v in a { if let Some(i) = v.as_str().and_then(|id| g.els.iter().position(|e| e.id == id)) { put(i); } }
    }
    for i in 0..g.els.len() { put(i); }
    (out, n)
}

/// The orderable elements of `g` in order, hidden ones included.
#[cfg(test)]
fn ordered(s: &Value, g: &'static Group) -> impl Iterator<Item = &'static El> {
    let (o, n) = order(s, g);
    (0..n).map(move |k| &g.els[o[k] as usize])
}

/// What the layout draws: the orderable elements of `g` that are shown, in order. The iterator owns a small copy of the
/// order (no borrow of the settings), so the caller may change the app while drawing.
pub(crate) fn visible(s: &Value, g: &'static Group) -> impl Iterator<Item = &'static El> {
    let (mut o, mut n) = order(s, g);
    let mut k = 0;
    for i in 0..n { if shown(s, g, g.els[o[i] as usize].id) { o[k] = o[i]; k += 1; } }
    n = k;
    (0..n).map(move |k| &g.els[o[k] as usize])
}

/// The rail entries the "…" menu offers instead: every hidden tab, Help and Settings.
pub(crate) fn rail_overflow(s: &Value) -> impl Iterator<Item = &'static El> + '_ {
    RAIL.els.iter().filter(move |e| !matches!(e.id, "separator" | "more") && !shown(s, &RAIL, e.id))
}

/// Moves the element at position `pos` of the order one step up or down (no-op at the ends).
pub(crate) fn move_el(s: &mut Value, g: &Group, pos: usize, up: bool) {
    let (mut o, n) = order(s, g);
    let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < n) };
    let Some(q) = other else { return };
    if pos >= n { return; }
    o.swap(pos, q);
    group_mut(s, g)["order"] = Value::Array(o[..n].iter().map(|&i| json!(g.els[i as usize].id)).collect());
}

/// Whether `g` looks as it does by default (nothing hidden, default order).
pub(crate) fn is_default(s: &Value, g: &Group) -> bool {
    let (o, n) = order(s, g);
    g.els.iter().all(|e| !hidden_raw(s, g, e.id)) && o[..n].windows(2).all(|w| w[0] < w[1])
}

/// "Reset": the group's defaults again.
pub(crate) fn reset(s: &mut Value, g: &Group) { if let Some(o) = s.as_object_mut() { o.remove(g.key); } }

/// The rail checkboxes of the former "App sidebar" page (`sb_*: false`) become hidden rail entries.
pub(crate) fn migrate_legacy(s: &mut Value) {
    for (old, id) in [("sb_messaging", "messaging"), ("sb_meetings", "meetings"), ("sb_calling", "calling"), ("sb_whiteboards", "whiteboards"), ("sb_contacts", "contacts")] {
        let Some(v) = s.as_object_mut().and_then(|o| o.remove(old)) else { continue };
        if v == false { set_shown(s, &RAIL, id, false); }
    }
}

impl crate::App {
    /// Whether element `id` of group `g` is drawn (Settings > Customize the interface).
    pub(crate) fn ui_shown(&self, g: &Group, id: &str) -> bool { shown(&self.settings, g, id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &Value, g: &'static Group) -> Vec<&'static str> { ordered(s, g).map(|e| e.id).collect() }

    #[test]
    fn defaults_show_everything_in_todays_order() {
        let s = json!({});
        for g in GROUPS {
            assert!(g.els.len() <= MAX && g.els.len() <= 32, "{}", g.key);
            for e in g.els { assert!(shown(&s, g, e.id), "{}.{}", g.key, e.id); }
            assert!(is_default(&s, g));
        }
        assert_eq!(ids(&s, &RAIL), ["messaging", "meetings", "calling", "separator", "whiteboards", "contacts", "teams", "more"]);
        assert_eq!(ids(&s, &COMPOSER), ["attach", "screenshot", "markdown", "emoji", "gif", "mention", "ai"]);
    }

    #[test]
    fn hide_show_and_reset() {
        let mut s = json!({ "theme": "dark" });
        assert!(set_shown(&mut s, &COMPOSER, "gif", false));
        assert!(!shown(&s, &COMPOSER, "gif") && shown(&s, &COMPOSER, "emoji"));
        assert!(shown(&s, &HEADER, "ai"), "groups are independent");
        assert!(!is_default(&s, &COMPOSER));
        set_shown(&mut s, &COMPOSER, "gif", false); // twice: still one entry
        assert_eq!(s["ui_composer"]["hidden"].as_array().unwrap().len(), 1);
        assert!(set_shown(&mut s, &COMPOSER, "gif", true));
        assert!(shown(&s, &COMPOSER, "gif"));
        set_shown(&mut s, &COMPOSER, "ai", false);
        reset(&mut s, &COMPOSER);
        assert!(shown(&s, &COMPOSER, "ai") && is_default(&s, &COMPOSER) && s["theme"] == "dark");
    }

    #[test]
    fn order_moves_and_survives_odd_files() {
        let mut s = json!({});
        move_el(&mut s, &RAIL, 0, true); // top can't go up
        assert!(is_default(&s, &RAIL));
        move_el(&mut s, &RAIL, 0, false);
        assert_eq!(&ids(&s, &RAIL)[..3], ["meetings", "messaging", "calling"]);
        move_el(&mut s, &RAIL, 7, false); // bottom can't go down
        assert_eq!(ids(&s, &RAIL).len(), 8);
        reset(&mut s, &RAIL);
        assert!(is_default(&s, &RAIL));
        // Unknown ids, repeats, fixed entries and missing ones: still every orderable entry exactly once.
        let s = json!({ "ui_rail": { "order": ["teams", "nope", "teams", "help", 5, "more"] } });
        assert_eq!(ids(&s, &RAIL), ["teams", "more", "messaging", "meetings", "calling", "separator", "whiteboards", "contacts"]);
    }

    #[test]
    fn settings_stay_reachable() {
        let mut s = json!({});
        assert!(set_shown(&mut s, &RAIL, "more", false));
        assert!(!can_hide(&s, &RAIL, "settings"));
        assert!(!set_shown(&mut s, &RAIL, "settings", false), "gear and … can't both go");
        assert!(shown(&s, &RAIL, "settings"));
        set_shown(&mut s, &RAIL, "more", true);
        assert!(set_shown(&mut s, &RAIL, "settings", false));
        assert!(!set_shown(&mut s, &RAIL, "more", false));
        // A hand-edited file with both hidden: the gear comes back.
        let s = json!({ "ui_rail": { "hidden": ["more", "settings"] } });
        assert!(shown(&s, &RAIL, "settings") && !shown(&s, &RAIL, "more"));
    }

    #[test]
    fn hidden_elements_are_not_drawn_and_the_order_is_applied() {
        let mut s = json!({});
        set_shown(&mut s, &RAIL, "calling", false);
        set_shown(&mut s, &RAIL, "separator", false);
        move_el(&mut s, &RAIL, 5, true); // contacts above whiteboards
        let drawn: Vec<_> = visible(&s, &RAIL).map(|e| e.id).collect();
        assert_eq!(drawn, ["messaging", "meetings", "contacts", "whiteboards", "teams", "more"]);
        assert_eq!(rail_overflow(&s).map(|e| e.id).collect::<Vec<_>>(), ["calling"], "the … menu offers the hidden tab");
        set_shown(&mut s, &COMPOSER, "ai", false);
        move_el(&mut s, &COMPOSER, 4, true); // GIF before emoji
        assert_eq!(visible(&s, &COMPOSER).map(|e| e.id).collect::<Vec<_>>(), ["attach", "screenshot", "markdown", "gif", "emoji", "mention"]);
    }

    #[test]
    fn no_combination_makes_settings_unreachable() {
        // Every subset of rail entries, hidden one after another through the setter (as the page does) or written straight into
        // the file: Settings is either on the rail or in the shown "…" menu.
        let ids: Vec<&str> = RAIL.els.iter().map(|e| e.id).collect();
        for mask in 0u32..(1 << ids.len()) {
            let pick: Vec<&str> = ids.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, id)| *id).collect();
            let mut s = json!({});
            for id in &pick { set_shown(&mut s, &RAIL, id, false); }
            let raw = json!({ "ui_rail": { "hidden": pick } });
            for s in [&s, &raw] {
                let gear = shown(s, &RAIL, "settings");
                let via_more = shown(s, &RAIL, "more") && rail_overflow(s).any(|e| e.id == "settings");
                assert!(gear || via_more, "mask {mask:b}");
            }
        }
    }

    #[test]
    fn old_sidebar_checkboxes_are_carried_over() {
        let mut s = json!({ "sb_calling": false, "sb_contacts": true });
        migrate_legacy(&mut s);
        assert!(!shown(&s, &RAIL, "calling") && shown(&s, &RAIL, "contacts"));
        assert!(s.get("sb_calling").is_none() && s.get("sb_contacts").is_none());
    }
}
