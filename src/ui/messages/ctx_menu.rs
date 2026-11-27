//! The message menu. A right click anywhere on a message row (text, header, avatar, pictures, videos, files, link cards, the
//! empty part of the row) opens the same entries as the hover toolbar's "···" button (`message_items`, in the Webex desktop
//! order), with the actions of what was clicked on top:
//!
//!   * text: Open Link / Copy link (on a link), Copy (with a selection), Select all;
//!   * a picture: Copy image, Save as; with the GIF Extras plugin on also Add to favorites / Remove from favorites;
//!   * a file or video: the file menu (Open, Download, Save as, Show in folder);
//!   * a link card: Open Link, Copy link.
//!
//! A deleted message a plugin keeps is gone on the service, so instead of the message's entries it only has "Copy
//! message". Plugins add their own entries at the end, after a separator (`message.menu`; a pick fires `message.menu_action`
//! at that plugin). While several messages are selected the whole menu is "Copy selected messages".
//!
//! The parts of a row note what was right-clicked (`note_hit`) while they are drawn; the row itself opens the menu at the end of
//! its frame (`row_right_click`), so a click that no part claims opens the plain message menu.
use super::*;
use crate::ui::widgets::tr;
use crate::extras::plugins::MenuItem as PluginItem;

/// What a right click on a message row landed on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Hit {
    /// The row outside any part with actions of its own (header, avatar, empty space, reactions).
    Row,
    /// The text; `link`: the web link under the pointer.
    Text { link: Option<String> },
    Image(Attachment),
    /// A file card or a video: its download key and name.
    File { key: String, name: String },
    /// A link-preview card: its address.
    Link(String),
}

fn hit_id(msg: &str) -> Id { Id::new(("msg-ctx-hit", msg)) }
/// The menu of message `msg` opened by a right click.
pub(super) fn ctx_key(msg: &str) -> String { format!("msg-ctx-{msg}") }
/// Its "Remind me in" submenu.
pub(super) fn remind_key(k: &str) -> String { format!("{k}-remind") }
/// Its "Add reaction" submenu.
pub(super) fn react_key(k: &str) -> String { format!("{k}-react") }
fn plug_id(k: &str) -> Id { Id::new(("msg-menu-plug", k)) }

/// The official client's reaction submenu (its "Reactions" entries in this order: Celebrate, Love, Like, Smile, Haha, Wow, Sad),
/// a check on the ones I gave; then "More reactions" for the emoji picker (Webox's own addition).
pub(super) const MENU_REACTIONS: [(&str, &str); 7] = [("celebrate", "Celebrate"), ("heart", "Love"), ("thumbsup", "Like"), ("smiley", "Smile"),
    ("haha", "Haha"), ("wow", "Wow"), ("sad", "Sad")];
pub(super) fn react_entries(m: &Message) -> Vec<menu::Entry> {
    let mut v: Vec<menu::Entry> = MENU_REACTIONS.iter().map(|(name, label)| {
        let mine = m.reactions.iter().any(|r| r.name == *name && r.mine.is_some());
        menu::Item::new(&format!("react:{name}"), tr(label)).checked(mine).entry()
    }).collect();
    v.push(menu::Entry::Sep);
    v.push(menu::Item::new("react-more", tr("More reactions")).icon("reactions-regular").entry());
    v
}

/// A part of message `msg` was right-clicked this frame: the row's menu offers that part's actions too.
pub(crate) fn note_hit(ctx: &egui::Context, msg: &str, hit: Hit) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(hit_id(msg), (pass, hit)));
}

/// What `note_hit` recorded for `msg` in this frame (the row itself otherwise).
pub(super) fn take_hit(ctx: &egui::Context, msg: &str) -> Hit {
    let pass = ctx.cumulative_pass_nr();
    let h = ctx.data(|d| d.get_temp::<(u64, Hit)>(hit_id(msg)));
    if h.is_some() { ctx.data_mut(|d| d.remove::<(u64, Hit)>(hit_id(msg))); }
    h.filter(|h| h.0 == pass).map_or(Hit::Row, |h| h.1)
}

/// Where the row `row` was right-clicked in this frame, if it was: the click ended on the row and nothing is on top of it there
/// (an open menu, the viewer, the hover toolbar's own layer).
pub(super) fn row_right_click(ui: &egui::Ui, row: Rect) -> Option<egui::Pos2> {
    let (clicked, pos) = ui.input(|i| (i.pointer.secondary_clicked(), i.pointer.interact_pos()));
    if !clicked { return None; }
    pos.filter(|p| row.contains(*p) && crate::ui::widgets::hover_ok(ui, row))
}

/// The entries of the right-click menu: the actions of what was clicked (`hit`), a separator, the message's own entries
/// (`msg_items`; a kept deleted message only "Copy message"), then the plugins' entries after a separator.
/// `selected`: there is a text selection in this message; `file_items`: the file menu, for a file hit; `img_fav`: on a picture,
/// the GIF Extras plugin's favourites entry (`Some(saved?)`, None while the plugin is off).
pub(super) fn context_entries(hit: &Hit, img_fav: Option<bool>, selected: bool, deleted: bool, file_items: Vec<menu::Entry>, msg_items: Vec<menu::Entry>, plugin: &[PluginItem]) -> Vec<menu::Entry> {
    let mut v: Vec<menu::Entry> = Vec::new();
    let link = |v: &mut Vec<menu::Entry>| {
        v.push(menu::Item::new("open-link", tr("Open Link")).icon("browser-regular").entry());
        v.push(menu::Item::new("copy-link", tr("Copy link")).icon("link-regular").entry());
    };
    match hit {
        Hit::Row => {}
        Hit::Text { link: l } => {
            if l.is_some() { link(&mut v); v.push(menu::Entry::Sep); }
            if selected { v.push(menu::Item::new("copy", tr("Copy")).icon("copy-regular").shortcut("Ctrl+C").entry()); }
            v.push(menu::Item::new("select-all", tr("Select all")).icon("select-all-regular").shortcut("Ctrl+A").entry());
        }
        Hit::Image(_) => {
            v.push(menu::Item::new("img-copy", tr("Copy image")).icon("copy-regular").entry());
            v.push(menu::Item::new("img-save", tr("Save as")).icon("save-regular").entry());
            if let Some(fav) = img_fav {
                let (label, icon) = if fav { ("Remove from favorites", "favorite-filled") } else { ("Add to favorites", "favorite-regular") };
                v.push(menu::Item::new("img-fav", tr(label)).icon(icon).entry());
            }
        }
        Hit::File { .. } => v.extend(file_items),
        Hit::Link(_) => link(&mut v),
    }
    if !v.is_empty() { v.push(menu::Entry::Sep); }
    if deleted { v.push(menu::Item::new("copy-message", tr("Copy message")).icon("copy-regular").entry()); } else { v.extend(msg_items); }
    v.extend(plugin_entries(plugin));
    tidy(v)
}

/// A menu never starts or ends with a separator, nor has two in a row (e.g. a part's actions with nothing after them).
pub(crate) fn tidy(v: Vec<menu::Entry>) -> Vec<menu::Entry> {
    let mut out: Vec<menu::Entry> = Vec::with_capacity(v.len());
    for e in v {
        let sep = matches!(e, menu::Entry::Sep);
        if sep && out.last().is_none_or(|l| matches!(l, menu::Entry::Sep)) { continue; }
        out.push(e);
    }
    while matches!(out.last(), Some(menu::Entry::Sep)) { out.pop(); }
    out
}

/// The plugins' entries (ids "plug:<index>"), after a separator; nothing without any.
pub(crate) fn plugin_entries(plugin: &[PluginItem]) -> Vec<menu::Entry> {
    if plugin.is_empty() { return vec![]; }
    let mut v = vec![menu::Entry::Sep];
    for (i, p) in plugin.iter().enumerate() {
        let mut it = menu::Item::new(&format!("plug:{i}"), p.label.clone());
        if let Some(ic) = icons::static_name(&p.icon) { it = it.icon(ic); }
        if p.danger { it = it.danger(); }
        v.push(it.entry());
    }
    v
}

impl App {
    /// The menu `k` of message `m` (in space `id`) was just opened: asks the plugins for their entries, once per opening.
    pub(super) fn message_menu_opened(&mut self, ctx: &egui::Context, id: &str, m: &Message, k: &str) {
        let payload = json!({ "space": id, "id": m.id, "deleted": m.deleted != 0, "edited": m.edited || !m.history.is_empty() || m.edit_tag, "mine": m.mine });
        let items = self.plugins.message_menu(&payload);
        ctx.data_mut(|d| d.insert_temp(plug_id(k), items));
    }

    /// Draws the open message menu `k` (the plugins' entries come with `items` already, see `context_entries`; the "···" menu
    /// adds them here with `add_plugins`) and its submenus: "Add reaction" opens the reaction picker beside its entry,
    /// "Remind me in" the reminder choices. Runs those and the plugins' entries itself; returns any other pick.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn message_menu_show(&mut self, ctx: &egui::Context, t: &Th, id: &str, m: &Message, k: &str, anchor: Rect, side: menu::Side, align: menu::Align, caret: bool,
        mut items: Vec<menu::Entry>, add_plugins: bool) -> Option<String> {
        let plug: Vec<PluginItem> = ctx.data(|d| d.get_temp(plug_id(k))).unwrap_or_default();
        if add_plugins && !msel(ctx, id).multi { items.extend(plugin_entries(&plug)); items = tidy(items); }
        let (kr, kx) = (remind_key(k), react_key(k));
        let mut out = None;
        if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, k, anchor, side, align, caret, &items) {
            if a != "remind" { menu::menu_close(ctx, &kr); }
            if a != "react" { menu::menu_close(ctx, &kx); }
            match a.as_str() {
                // The submenus keep the main menu open.
                "react" => { menu::menu_open(ctx, k); menu::menu_close(ctx, &kr); menu::menu_open(ctx, &kx); }
                "remind" => { menu::menu_open(ctx, k); menu::menu_close(ctx, &kx); menu::menu_open(ctx, &kr); }
                p if p.starts_with("plug:") => {
                    if let Some(it) = p["plug:".len()..].parse::<usize>().ok().and_then(|i| plug.get(i)) { self.plugins.message_menu_action(&it.plugin, id, &m.id, &it.id); }
                }
                _ => out = Some(a),
            }
        }
        if menu::menu_is_open(ctx, &kr) {
            let at = menu::menu_item_rect(ctx, k, "remind").unwrap_or(anchor);
            if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, &kr, at, menu::Side::Right, menu::Align::Left, false, &crate::ui::msg_remind::entries()) {
                menu::menu_close(ctx, k);
                self.remind_pick(id, m, &a, ctx.input(|i| i.time));
            }
        }
        if menu::menu_is_open(ctx, &kx) {
            let at = menu::menu_item_rect(ctx, k, "react").unwrap_or(anchor);
            if let Some(a) = menu::webex_menu(ctx, &mut self.icons, t, &kx, at, menu::Side::Right, menu::Align::Left, false, &react_entries(m)) {
                menu::menu_close(ctx, k);
                match a.strip_prefix("react:") {
                    Some(name) => self.toggle_reaction(&m.id, name),
                    None => self.open_reaction_picker(ctx, &m.id, at), // "More reactions"
                }
            }
        }
        out
    }

    /// Screenshot checks only: opens the right-click menu of `m` in the middle of the window as if its text was right-clicked.
    /// With `WEBOX_SHOT_CTX_HIT=image` as if its first picture was right-clicked.
    pub(super) fn shot_context_menu(&mut self, ctx: &egui::Context, id: &str, m: &Message) {
        let k = ctx_key(&m.id);
        let at = ctx.screen_rect().center();
        let hit = match (std::env::var("WEBOX_SHOT_CTX_HIT").as_deref(), m.images.first()) { (Ok("image"), Some(a)) => Hit::Image(a.clone()), _ => Hit::Text { link: None } };
        ctx.data_mut(|d| { d.insert_temp(Id::new(("msg-ctx-target", &m.id)), hit); d.insert_temp(Id::new(("msg-ctx-at", &m.id)), at); });
        menu::menu_open(ctx, &k);
        self.message_menu_opened(ctx, id, m, &k);
    }

    /// The right-click menu of a message row, at the end of the row's frame: opens it on a right click anywhere on `row` (with
    /// what `note_hit` recorded), then draws it while it is open. `text_origin`: where the row's text galley was drawn.
    pub(super) fn row_context_menu(&mut self, ui: &egui::Ui, t: &Th, id: &str, m: &Message, ct: &Content, row: Rect) {
        let ctx = ui.ctx().clone();
        let k = ctx_key(&m.id);
        let (k_hit, k_at) = (Id::new(("msg-ctx-target", &m.id)), Id::new(("msg-ctx-at", &m.id)));
        if let Some(at) = row_right_click(ui, row) {
            let hit = take_hit(&ctx, &m.id);
            ctx.data_mut(|d| { d.insert_temp(k_hit, hit); d.insert_temp(k_at, at); });
            menu::menu_close(&ctx, &remind_key(&k));
            menu::menu_close(&ctx, &react_key(&k));
            menu::menu_open(&ctx, &k);
            self.message_menu_opened(&ctx, id, m, &k);
        } else {
            let _ = take_hit(&ctx, &m.id); // a part noted a click the row did not take (e.g. covered): forget it
        }
        if ![k.clone(), remind_key(&k), react_key(&k)].iter().any(|x| menu::menu_is_open(&ctx, x)) { return; }
        let hit: Hit = ctx.data(|d| d.get_temp(k_hit)).unwrap_or(Hit::Row);
        let at = ctx.data(|d| d.get_temp::<egui::Pos2>(k_at)).unwrap_or(row.min);
        // The text selection of this message (Copy) and the text for Select all.
        let k_sel = Id::new("msg-sel");
        let text = ct.text.as_ref().map(|g| g.job.text.clone()).unwrap_or_default();
        let total = text.chars().count();
        let sel = ctx.data(|d| d.get_temp::<(String, usize, usize)>(k_sel)).filter(|s| s.0 == m.id && s.1 != s.2).map(|s| (s.1.min(s.2), s.1.max(s.2)));
        let selected = sel.map(|(a, b)| selected_text(&text, &ct.emojis, &ct.links, a, b));
        let items = if msel(&ctx, id).multi { self.message_entries(&ctx, id, m) } else {
            let file_items = match &hit { Hit::File { key, .. } => self.file_menu_entries(key, false), _ => vec![] };
            let plug: Vec<PluginItem> = ctx.data(|d| d.get_temp(plug_id(&k))).unwrap_or_default();
            let img_fav = match &hit { Hit::Image(a) => self.gif_ext_image_menu(&ctx, a), _ => None };
            context_entries(&hit, img_fav, selected.is_some(), m.deleted != 0, file_items, self.message_entries(&ctx, id, m), &plug)
        };
        let Some(a) = self.message_menu_show(&ctx, t, id, m, &k, Rect::from_min_size(at, Vec2::ZERO), menu::Side::Below, menu::Align::Left, false, items, false) else { return };
        let link = match &hit { Hit::Text { link } => link.clone(), Hit::Link(u) => Some(u.clone()), _ => None };
        let now = ctx.input(|i| i.time);
        match (a.as_str(), &hit) {
            ("copy", _) => if let Some(s) = &selected { ctx.copy_text(s.clone()); },
            ("select-all", _) => ctx.data_mut(|d| d.insert_temp(k_sel, (m.id.clone(), 0usize, total))),
            ("img-copy", Hit::Image(a)) => self.image_copy(&ctx, &a.key),
            ("img-save", Hit::Image(a)) => { let a = a.clone(); self.image_save(&a.key, Some(&a)); }
            ("img-fav", Hit::Image(a)) => { let a = a.clone(); self.gif_ext_image_menu_pick(&ctx, &a); }
            (f, Hit::File { key, name }) if f.starts_with("file-") => { let (key, name) = (key.clone(), name.clone()); self.file_menu_do(f, &key, &name, now); }
            _ => self.message_do(&ctx, id, m, &a, link.as_deref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids of the entries ("-" for a separator).
    fn ids(v: &[menu::Entry]) -> Vec<String> {
        v.iter().map(|e| match e { menu::Entry::Item(i) => i.id.clone(), menu::Entry::Sep => "-".into(), _ => "?".into() }).collect()
    }
    fn labels(v: &[menu::Entry]) -> Vec<String> { v.iter().filter_map(|e| match e { menu::Entry::Item(i) => Some(i.label.clone()), _ => None }).collect() }
    fn msg(mine: bool) -> Message { Message { id: "m1".into(), text: "Hello".into(), mine, ..Default::default() } }

    #[test]
    fn the_message_entries_follow_the_webex_menu() {
        assert_eq!(labels(&crate::ui::msg_actions::message_items(&msg(true), false)), [tr("Copy message"), tr("Edit message"), tr("Start a thread"),
            tr("Add reaction"), tr("Forward message"), tr("Remind me in"), tr("Pin message"), tr("Quote message"), tr("Translate message"),
            tr("Flag message"), tr("Delete message")]);
        let other = crate::ui::msg_actions::message_items(&Message { parent: "root".into(), ..msg(false) }, true);
        assert_eq!(labels(&other), [tr("Copy message"), tr("Reply to thread"), tr("Add reaction"), tr("Forward message"), tr("Remind me in"),
            tr("Pin message"), tr("Quote message"), tr("Translate message"), tr("Unflag message"), tr("Mark as unread")]);
        let sub: Vec<&str> = other.iter().filter_map(|e| match e { menu::Entry::Item(i) if i.submenu => Some(i.id.as_str()), _ => None }).collect();
        assert_eq!(sub, ["react", "remind"]);
    }

    #[test]
    fn the_reaction_submenu_lists_the_official_reactions() {
        let m = Message { reactions: vec![cloud::Reaction { name: "heart".into(), count: 1, mine: Some("r1".into()), ..Default::default() }], ..msg(false) };
        let v = react_entries(&m);
        assert_eq!(labels(&v), [tr("Celebrate"), tr("Love"), tr("Like"), tr("Smile"), tr("Haha"), tr("Wow"), tr("Sad"), tr("More reactions")]);
        let checked: Vec<String> = v.iter().filter_map(|e| match e { menu::Entry::Item(i) if i.checked => Some(i.id.clone()), _ => None }).collect();
        assert_eq!(checked, ["react:heart"]);
    }

    #[test]
    fn each_part_of_a_row_puts_its_actions_on_top() {
        let base = || crate::ui::msg_actions::message_items(&msg(false), false);
        let n = base().len();
        let tail = ids(&base());
        let with = |hit: Hit, sel: bool| ids(&context_entries(&hit, None, sel, false, vec![menu::Item::new("file-open", tr("Open")).entry()], base(), &[]));
        // The empty part of the row, the header, the avatar: exactly the "···" menu.
        assert_eq!(with(Hit::Row, false), tail);
        let top = |v: Vec<String>| v[..v.len() - n].to_vec();
        assert_eq!(top(with(Hit::Text { link: None }, false)), ["select-all", "-"]);
        assert_eq!(top(with(Hit::Text { link: None }, true)), ["copy", "select-all", "-"]);
        assert_eq!(top(with(Hit::Text { link: Some("https://x.example".into()) }, false)), ["open-link", "copy-link", "-", "select-all", "-"]);
        assert_eq!(top(with(Hit::Image(Attachment::default()), false)), ["img-copy", "img-save", "-"]);
        assert_eq!(top(with(Hit::File { key: "k".into(), name: "a.pdf".into() }, false)), ["file-open", "-"]);
        assert_eq!(top(with(Hit::Link("https://x.example".into()), false)), ["open-link", "copy-link", "-"]);
        for h in [Hit::Row, Hit::Text { link: None }, Hit::Image(Attachment::default()), Hit::Link("u".into())] { assert!(with(h, false).ends_with(&tail)); }
    }

    #[test]
    fn a_picture_gets_the_favourites_entry_only_from_the_gif_extras_plugin() {
        let img = Hit::Image(Attachment { key: "k".into(), mime: "image/png".into(), ..Default::default() });
        let top = |fav: Option<bool>| { let v = context_entries(&img, fav, false, false, vec![], vec![], &[]); (ids(&v), labels(&v)) };
        // The plugin off (no entry from it): the menu is exactly as before.
        assert_eq!(top(None).0, ["img-copy", "img-save"]);
        let (i, l) = top(Some(false));
        assert_eq!(i, ["img-copy", "img-save", "img-fav"]);
        assert_eq!(l[2], tr("Add to favorites"));
        assert_eq!(top(Some(true)).1[2], tr("Remove from favorites"));
        // Only on a picture.
        for h in [Hit::Row, Hit::Text { link: None }, Hit::Link("u".into()), Hit::File { key: "k".into(), name: "a.pdf".into() }] {
            assert!(!ids(&context_entries(&h, Some(false), false, false, vec![], vec![], &[])).contains(&"img-fav".to_string()));
        }
    }

    #[test]
    fn a_kept_deleted_message_copies_and_shows_the_plugins_entries() {
        let plug = [PluginItem { plugin: "dev.x".into(), id: "forget".into(), label: "Remove deleted message".into(), icon: "delete-regular".into(), danger: true },
            PluginItem { plugin: "dev.x".into(), id: "clear".into(), label: "Clear this space's history".into(), icon: "no-such-icon".into(), danger: false }];
        let v = context_entries(&Hit::Text { link: None }, None, false, true, vec![], crate::ui::msg_actions::message_items(&msg(true), false), &plug);
        assert_eq!(ids(&v), ["select-all", "-", "copy-message", "-", "plug:0", "plug:1"]);
        let menu::Entry::Item(p0) = &v[4] else { panic!() };
        assert_eq!((p0.label.as_str(), p0.icon, p0.danger), ("Remove deleted message", Some("delete-regular"), true));
        let menu::Entry::Item(p1) = &v[5] else { panic!() };
        assert_eq!(p1.icon, None, "an unknown icon name draws no icon");
        // A picture of a kept deleted message still has its picture actions.
        assert_eq!(ids(&context_entries(&Hit::Image(Attachment::default()), None, false, true, vec![], vec![], &[])), ["img-copy", "img-save", "-", "copy-message"]);
        // No plugin: no separator, nothing added.
        assert!(plugin_entries(&[]).is_empty());
    }

    #[test]
    fn a_menu_never_ends_in_a_separator() {
        // A kept deleted message right-clicked on its text, no plugin entries: the text actions, one separator, "Copy message".
        assert_eq!(ids(&context_entries(&Hit::Text { link: None }, None, false, true, vec![], vec![], &[])), ["select-all", "-", "copy-message"]);
        // Whatever the parts give, no separator is left at either end or doubled.
        let sep = || menu::Entry::Sep;
        let it = |id: &str| menu::Item::new(id, id).entry();
        assert_eq!(ids(&tidy(vec![sep(), it("a"), sep(), sep(), it("b"), sep()])), ["a", "-", "b"]);
        assert!(tidy(vec![sep(), sep()]).is_empty());
        assert_eq!(ids(&context_entries(&Hit::Text { link: None }, None, false, false, vec![], vec![sep()], &[])), ["select-all"]);
    }

    /// A row with a text part and a picture part, right-clicked at `at`: what the row's menu would open with.
    fn right_click_on(at: egui::Pos2) -> Option<Hit> {
        let ctx = egui::Context::default();
        let row = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(400.0, 200.0));
        let (text, img) = (Rect::from_min_size(egui::pos2(60.0, 10.0), Vec2::new(200.0, 20.0)), Rect::from_min_size(egui::pos2(60.0, 40.0), Vec2::new(120.0, 80.0)));
        let mut out = None;
        let frame = |events: Vec<egui::Event>, out: &mut Option<Hit>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let (r, _) = ui.allocate_exact_size(row.size(), Sense::hover());
                    let tr = ui.interact(text.translate(r.min.to_vec2()), Id::new("t"), Sense::click_and_drag());
                    if tr.secondary_clicked() { note_hit(ui.ctx(), "m1", Hit::Text { link: None }); }
                    let ir = ui.interact(img.translate(r.min.to_vec2()), Id::new("i"), Sense::click());
                    if ir.secondary_clicked() { note_hit(ui.ctx(), "m1", Hit::Image(Attachment { key: "img".into(), ..Default::default() })); }
                    if row_right_click(ui, r).is_some() { *out = Some(take_hit(ui.ctx(), "m1")); } else { let _ = take_hit(ui.ctx(), "m1"); }
                });
            });
        };
        let btn = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(at)], &mut out);
        frame(vec![btn(true)], &mut out);
        frame(vec![btn(false)], &mut out);
        out
    }

    /// An edited message with logged history: earlier-text lines above the text and the clickable "(edited)" tag below it
    /// (a click-sensing widget, as `edit_tag_click` adds), right-clicked at `at`.
    fn right_click_on_edited(at: egui::Pos2) -> Option<Hit> {
        let ctx = egui::Context::default();
        let row = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(400.0, 120.0));
        let text = Rect::from_min_size(egui::pos2(60.0, 50.0), Vec2::new(200.0, 20.0));
        let tag = Rect::from_min_size(egui::pos2(60.0, 74.0), Vec2::new(70.0, 14.0));
        let mut out = None;
        let frame = |events: Vec<egui::Event>, out: &mut Option<Hit>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let (r, _) = ui.allocate_exact_size(row.size(), Sense::hover());
                    let tr = ui.interact(text.translate(r.min.to_vec2()), Id::new("t"), Sense::click_and_drag());
                    if tr.secondary_clicked() { note_hit(ui.ctx(), "m1", Hit::Text { link: None }); }
                    let _ = ui.interact(tag.translate(r.min.to_vec2()), Id::new(("msg-edit-tag", "s", "m1", (0u32, 1u8))), Sense::click());
                    if row_right_click(ui, r).is_some() { *out = Some(take_hit(ui.ctx(), "m1")); } else { let _ = take_hit(ui.ctx(), "m1"); }
                });
            });
        };
        let btn = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(at)], &mut out);
        frame(vec![btn(true)], &mut out);
        frame(vec![btn(false)], &mut out);
        out
    }

    #[test]
    fn an_edited_message_with_history_opens_its_menu_everywhere() {
        assert_eq!(right_click_on_edited(egui::pos2(350.0, 60.0)), Some(Hit::Row), "the empty right side");
        assert_eq!(right_click_on_edited(egui::pos2(100.0, 20.0)), Some(Hit::Row), "an earlier-text line");
        assert_eq!(right_click_on_edited(egui::pos2(80.0, 80.0)), Some(Hit::Row), "the (edited) tag");
        assert_eq!(right_click_on_edited(egui::pos2(100.0, 60.0)), Some(Hit::Text { link: None }), "the text");
    }

    #[test]
    fn a_right_click_anywhere_on_the_row_opens_its_menu() {
        assert_eq!(right_click_on(egui::pos2(100.0, 20.0)), Some(Hit::Text { link: None }));
        assert_eq!(right_click_on(egui::pos2(100.0, 80.0)), Some(Hit::Image(Attachment { key: "img".into(), ..Default::default() })));
        assert_eq!(right_click_on(egui::pos2(350.0, 150.0)), Some(Hit::Row), "the empty part of the row");
        assert_eq!(right_click_on(egui::pos2(20.0, 20.0)), Some(Hit::Row), "the avatar / header");
        assert_eq!(right_click_on(egui::pos2(500.0, 20.0)), None, "outside the row");
    }
}
