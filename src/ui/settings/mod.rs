#![allow(clippy::too_many_arguments)]
//! The Settings page (`Tab::Settings`), shown inside the main window like the other pages: the rail stays, the content area is
//! replaced by two rounded gradient cards (4 px gap) - the settings navigation on the left, the page on the right. Rail gear icon /
//! Ctrl+, switch to it; Esc (or any rail item) leaves it and the previous tab / space comes back (history back).
//! Navigation: rows 36 high, spacing 2, radius 4, 20 px icon, bold chevron for parents; page: header with the title, body
//! margins 16 / 32, footer "Reset to default" where the page has a reset.
//! Search: a 32 high field, results popup radius 12, 500 ms debounce, rows with text / description / path.
//! Screenshot env: `WEBOX_SHOT_SETTINGS=<page>` (general, audio, smartaudio, video, share, notifications, messages, calls, notifmeetings,
//! appearance, accessibility, shortcuts, privacy, messaging, meetings, join, layout, lobby, calling, dock, block, devices, accessories, customize (or sidebar)),
//! optional `WEBOX_SHOT_SETTINGS_SEARCH=<text>`.
use crate::*;

pub(crate) mod kit;
mod customize_page;
pub(crate) mod nav;
pub(crate) mod pages_a;
mod plugins_page;
mod pages_b;
mod sound_ui;
pub(crate) mod video_page;

use nav::Page;
pub(crate) use pages_a::msg_font_scale_of;

const NAV_W: f32 = 280.0; // navigation column width
fn header_h() -> f32 { crate::ui::theme::density::m().settings_header_h } // page header height (52)
const CONTENT_W: f32 = 720.0; // page body width cap

#[derive(Clone)]
struct SUi { page: Page, open: Vec<Page>, search: String, edited: f64, applied: String, hi: usize }
impl Default for SUi { fn default() -> SUi { SUi { page: Page::General, open: vec![], search: String::new(), edited: 0.0, applied: String::new(), hi: 0 } } }

impl App {
    /// Message-font scale of Accessibility > Font size (1.0 = Regular). Used by the message list for body text.
    #[allow(dead_code)]
    pub(crate) fn msg_font_scale(&self) -> f32 { msg_font_scale_of(self.settings["msg_font_size"].as_str().unwrap_or("regular")) }

    /// The active theme: settings key `theme` = the theme id ("MomentumDark" default, "MomentumDefault", ...; the older
    /// Webox values "dark" / "light" / "system" are still read). While Windows high contrast is on, the "OS High Contrast" theme
    /// replaces the choice (the picker is disabled then).
    pub(crate) fn theme_id(&self, ctx: &egui::Context) -> ThemeId {
        if crate::ui::theme::hc_cached(ctx) { return ThemeId::HighContrast; }
        match self.settings["theme"].as_str() {
            Some("system") => if ctx.system_theme() == Some(egui::Theme::Light) { ThemeId::Light } else { ThemeId::Dark },
            Some(s) => ThemeId::from_key(s).unwrap_or_default(),
            None => ThemeId::Dark,
        }
    }
    /// Whether a light theme is active.
    #[allow(dead_code)]
    pub(crate) fn light_mode(&self, ctx: &egui::Context) -> bool { !self.theme_id(ctx).is_dark() }
    /// The theme of call windows / call toasts: "Always use dark theme" (`always_dark_calls`) keeps them on the dark
    /// variant of the selected theme.
    pub(crate) fn call_theme(&self, ctx: &egui::Context) -> Th {
        let id = self.theme_id(ctx);
        Th::of(if self.settings["always_dark_calls"] == true { id.dark_variant() } else { id })
    }

    /// Per-frame hook (frame.rs): collects finished file dialogs; in --screenshot runs opens the page named by WEBOX_SHOT_SETTINGS.
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context, _t: &Th, now: f64) {
        if self.shot.is_some() && self.frames == 2 {
            if let Ok(n) = std::env::var("WEBOX_SHOT_SETTINGS") {
                self.set_tab(Tab::Settings);
                if let Some(p) = nav::by_name(&n) { self.with_sui(ctx, |s| { s.page = p; if let Some(par) = nav::parent(p) { s.open.push(par); } }); }
                if let Ok(q) = std::env::var("WEBOX_SHOT_SETTINGS_SEARCH") { self.with_sui(ctx, |s| { s.search = q.clone(); s.applied = q; }); }
            }
        }
        self.poll_picks(now);
        self.repo_tick(ctx, now); // plugin repositories: finished downloads / index fetches, the daily check (plugins_page/)
    }
    fn with_sui<R>(&self, ctx: &egui::Context, f: impl FnOnce(&mut SUi) -> R) -> R {
        let id = egui::Id::new("settings-ui");
        let mut s: SUi = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        let r = f(&mut s);
        ctx.data_mut(|d| d.insert_temp(id, s));
        r
    }

    /// The whole Settings tab (called from frame.rs in place of the generic card).
    pub(crate) fn settings_tab(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        egui::CentralPanel::default().frame(egui::Frame::default().inner_margin(egui::Margin { left: 0, right: 4, top: 0, bottom: 0 })).show(ctx, |ui| {
            let full = ui.available_rect_before_wrap();
            let _ = ui.allocate_rect(full, Sense::hover());
            let (nav_r, page_r) = (Rect::from_min_max(full.min, pos2(full.left() + NAV_W, full.bottom())), Rect::from_min_max(pos2(full.left() + NAV_W + 4.0, full.top()), full.max));
            vgrad(ui.painter(), nav_r, 10.0, t.card, t.card_b); crate::ui::theme::decor::card(ui.painter(), nav_r, 10.0, true);
            vgrad(ui.painter(), page_r, 10.0, t.card, t.card_b); crate::ui::theme::decor::card(ui.painter(), page_r, 10.0, false);
            let mut s: SUi = ctx.data(|d| d.get_temp(egui::Id::new("settings-ui"))).unwrap_or_default();
            if let Some(p) = ctx.data_mut(|d| d.remove_temp::<Page>(egui::Id::new("settings-goto"))) { s.page = p; if let Some(par) = nav::parent(p) { if !s.open.contains(&par) { s.open.push(par); } } }
            self.settings_nav(ui, t, &mut s, nav_r);
            self.settings_content(ui, t, &mut s, page_r, now);
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) { // combo boxes / search consume Esc before this runs (they are drawn first)
                if s.search.is_empty() { self.act(Act::Back, ctx); } else { s.search.clear(); s.applied.clear(); }
            }
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("settings-ui"), s));
        });
        if ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("settings-signout"))).unwrap_or(false) { self.sign_out(); }
    }

    fn settings_nav(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut SUi, nav_rect: Rect) {
        let k = kit::tok(t);
        let sf = Rect::from_min_size(nav_rect.min + Vec2::new(12.0, 12.0), Vec2::new(NAV_W - 24.0, 32.0));
        let dm = crate::ui::theme::density::m(); // navigation rows: Standard top 56, rows 36 on a 38 pitch
        let mut y = nav_rect.top() + dm.settings_nav_top;
        for (p, title, icon, depth) in nav::NAV {
            if let Some(par) = nav::parent(p) { if !s.open.contains(&par) { continue; } }
            let r = Rect::from_min_size(pos2(nav_rect.left() + 8.0, y), Vec2::new(NAV_W - 16.0, dm.settings_nav_row));
            y += dm.settings_nav_pitch;
            let resp = ui.interact(r, egui::Id::new(("settings-nav", p as usize)), Sense::click());
            let sel = s.page == p;
            if sel { ui.painter().rect_filled(r, 4.0, k.item_pressed); if crate::ui::theme::decor::on(collabhost_plugin_api::deco::NAV_UNDERLINE) { crate::ui::theme::decor::fill(ui.painter(), Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 3.0), pos2(r.right() - 8.0, r.bottom())), 1.5, &crate::ui::theme::decor::stripes(), 0.0, 1.0); } } else if resp.hovered() { ui.painter().rect_filled(r, 4.0, k.item_hover); }
            let x0 = r.left() + 8.0 + if depth > 0 { 28.0 } else { 0.0 };
            if !icon.is_empty() { self.icons.paint(ui, icon, Rect::from_center_size(pos2(x0 + 10.0, r.center().y), Vec2::splat(20.0)), t.text1); }
            let tx = x0 + if icon.is_empty() { 0.0 } else { 28.0 };
            let col = if sel || depth == 0 { t.text1 } else { t.text2 };
            crate::ui::chrome::fit_text(ui.painter(), tx, r.center().y, tr(title), if sel { semibold(14.0) } else { FontId::proportional(14.0) }, col, r.right() - tx - 30.0);
            if nav::has_children(p) { self.icons.paint(ui, if s.open.contains(&p) { "arrow-down-bold" } else { "arrow-right-bold" }, Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), Vec2::splat(16.0)), col); }
            if resp.clicked() {
                s.page = p;
                if nav::has_children(p) { if let Some(i) = s.open.iter().position(|x| *x == p) { s.open.remove(i); } else { s.open.push(p); } }
            }
        }
        self.settings_search(ui, t, s, sf, ui.ctx().input(|i| i.time));
    }

    fn settings_content(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut SUi, pr: Rect, now: f64) {
        ui.painter().text(pr.left_top() + Vec2::new(24.0, 10.0 + 18.0), egui::Align2::LEFT_CENTER, nav::title(s.page), semibold(20.0), t.text1);
        let mut top = pr.top() + header_h() + 8.0;
        // Pinned content: the Video page's camera preview stays above the scrolling body.
        if s.page == Page::Video {
            let r = Rect::from_min_size(pos2(pr.left() + 24.0, top), video_page::PREVIEW);
            self.video_preview(ui, t, now, r);
            top += video_page::PREVIEW.y + video_page::PINNED_GAP;
        }
        let body = Rect::from_min_max(pos2(pr.left() + 24.0, top), pos2(pr.right() - 8.0, pr.bottom() - 8.0));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        bui.set_clip_rect(body);
        bui.spacing_mut().item_spacing.y = 0.0;
        let page = s.page;
        crate::ui::widgets::vscroll().id_salt(("settings-scroll", page as usize)).auto_shrink([false, false]).show(&mut bui, |ui| {
            ui.set_width((body.width() - 24.0).min(CONTENT_W));
            ui.add_space(4.0);
            self.settings_page_body(ui, t, now, page);
            ui.add_space(24.0);
        });
    }
    fn settings_page_body(&mut self, ui: &mut egui::Ui, t: &Th, now: f64, page: Page) {
        match page {
            Page::General => self.page_general(ui, t, now),
            Page::Audio => self.page_audio(ui, t, now),
            Page::SmartAudio => self.page_smart_audio(ui, t),
            Page::Video => self.page_video(ui, t),
            Page::Share => self.page_share(ui, t),
            Page::Notifications => self.page_notifications(ui, t, now),
            Page::NotifMessages => self.page_notif_messages(ui, t, now),
            Page::NotifCalls => self.page_notif_calls(ui, t, now),
            Page::NotifMeetings => self.page_notif_meetings(ui, t, now),
            Page::Appearance => self.page_appearance(ui, t),
            Page::Accessibility => self.page_accessibility(ui, t),
            Page::Shortcuts => self.shortcuts_page(ui, t),
            Page::Privacy => self.page_privacy(ui, t, now),
            Page::Messaging => self.page_messaging(ui, t),
            Page::Meetings => self.page_meetings(ui, t, now),
            Page::MeetJoin => self.page_meet_join(ui, t),
            Page::MeetLayout => self.page_meet_layout(ui, t),
            Page::MeetLobby => self.page_meet_lobby(ui, t, now),
            Page::Calling => self.page_calling(ui, t),
            Page::CallDock => self.page_call_dock(ui, t),
            Page::CallBlock => self.page_call_block(ui, t, now),
            Page::Devices => self.page_devices(ui, t, now),
            Page::Accessories => self.page_accessories(ui, t),
            Page::Customize => self.page_customize(ui, t, now),
            Page::Plugins => self.page_plugins(ui, t, now),
        }
    }

    /// Audio page device block: the call module's microphone / speaker pickers with level meter (real devices).
    pub(crate) fn settings_audio_devices(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) { self.device_pickers(ui, t, now); }

    /// Settings search: field on top of the navigation, results in a popup below it (500 ms debounce, Enter searches at once).
    fn settings_search(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut SUi, sf: Rect, now: f64) {
        let k = kit::tok(t);
        let id = egui::Id::new("settings-search");
        ui.painter().rect_filled(sf, 8.0, k.input_bg);
        let focused = ui.memory(|m| m.has_focus(id));
        ui.painter().rect_stroke(sf, 8.0, Stroke::new(if focused { 2.0_f32 } else { 1.0_f32 }, if focused { k.focus } else { k.outline }), egui::StrokeKind::Inside);
        self.icons.paint(ui, "search-regular", Rect::from_center_size(sf.left_center() + Vec2::new(16.0, 0.0), Vec2::splat(16.0)), t.text2);
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(sf.min + Vec2::new(32.0, 0.0), sf.max - Vec2::new(28.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        
        let r = c.add(egui::TextEdit::singleline(&mut s.search).id(id).frame(false).margin(Vec2::ZERO).font(FontId::proportional(14.0)).hint_text(RichText::new(tr(SEARCH_HINT)).color(t.text2).size(14.0)).desired_width(sf.width() - 60.0));
        if r.changed() { s.edited = now; }
        if r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) { s.applied = s.search.clone(); }
        if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) { s.search.clear(); s.applied.clear(); }
        if !s.search.is_empty() {
            let xr = Rect::from_center_size(sf.right_center() - Vec2::new(14.0, 0.0), Vec2::splat(20.0));
            let xs = ui.interact(xr, egui::Id::new("settings-search-x"), Sense::click());
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(11.0)), t.text2);
            if xs.clicked() { s.search.clear(); s.applied.clear(); }
        }
        if s.search != s.applied && now - s.edited >= 0.5 { s.applied = s.search.clone(); }
        if s.search.is_empty() { s.applied.clear(); }
        if s.applied.is_empty() || s.applied != s.search { return; }
        // The fixed index plus every plugin's own settings (from their schemas; a hit opens that plugin's page).
        let mut res: Vec<(Page, String, String, String, Option<String>)> = nav::search(&s.applied).into_iter()
            .map(|(p, l, d)| (p, l.to_string(), d.to_string(), format!("{} › {}", tr("Settings"), nav::title(p)), None)).collect();
        res.extend(self.plugins.search_settings(&s.applied).into_iter().map(|h| {
            let d = if h.description.is_empty() { h.plugin.clone() } else { h.description };
            (Page::Plugins, h.label, d, format!("{} › {} › {}", tr("Settings"), tr("Plugins"), h.plugin), Some(h.key))
        }));
        let row_h = 60.0;
        let h = if res.is_empty() { 80.0 } else { (res.len() as f32 * row_h + 20.0).min(300.0) };
        let pop = Rect::from_min_size(sf.left_bottom() + Vec2::new(0.0, 4.0), Vec2::new(sf.width() + 40.0, h)); // widened a little so labels fit
        let mut goto = None;
        egui::Area::new(egui::Id::new("settings-search-pop")).order(egui::Order::Tooltip).fixed_pos(pop.min).show(ui.ctx(), |ui| {
            ui.painter().rect_filled(pop, 12.0, k.popup_bg);
            ui.painter().rect_stroke(pop, 12.0, Stroke::new(1.0_f32, k.border), egui::StrokeKind::Inside);
            ui.allocate_rect(pop, Sense::hover());
            if res.is_empty() { ui.painter().text(pop.center(), egui::Align2::CENTER_CENTER, tr("No matching settings found"), FontId::proportional(14.0), t.text1); return; }
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(pop.shrink(8.0)));
            crate::ui::widgets::vscroll().id_salt("settings-search-scroll").auto_shrink([false, false]).show(&mut inner, |ui| {
                for (i, (p, label, desc, path, key)) in res.iter().enumerate() {
                    let (rr, rs) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h - 4.0), Sense::click());
                    if rs.hovered() { s.hi = i; }
                    if s.hi == i && rs.hovered() { ui.painter().rect_filled(rr, 6.0, k.item_hover); }
                    let pad = rr.left() + 12.0;
                    crate::ui::chrome::fit_text(ui.painter(), pad, rr.top() + 12.0, label, FontId::proportional(14.0), t.text1, rr.width() - 24.0);
                    crate::ui::chrome::fit_text(ui.painter(), pad, rr.top() + 28.0, desc, FontId::proportional(12.0), t.text2, rr.width() - 24.0);
                    crate::ui::chrome::fit_text(ui.painter(), pad, rr.top() + 43.0, path, FontId::proportional(11.0), alpha(t.text2, 0.8), rr.width() - 24.0);
                    if rs.clicked() { goto = Some((*p, key.clone())); }
                }
            });
        });
        if let Some((p, key)) = goto {
            if let Some(k) = key { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(plugins_page::GOTO), k)); }
            s.page = p; if let Some(par) = nav::parent(p) { if !s.open.contains(&par) { s.open.push(par); } } s.search.clear(); s.applied.clear(); }
    }

    /// Kept for `pages.rs` (Tab::Settings is intercepted in `settings_window`, this only runs if something draws the tab directly).
    pub(crate) fn settings_page(&mut self, _ui: &mut egui::Ui, _t: &Th, _now: f64) {} // drawn by settings_tab (frame.rs)
}

const SEARCH_HINT: &str = "Search the settings"; // Webox wording: the pack's "Search settings" reads "Sucheinstellungen" in German

/// Calling > Calling dock > "Turn on the calling dock" (bool, default true).
pub(crate) const DOCK_ENABLED: &str = "calling_dock";
/// Calling > Calling dock > "Keep the calling dock in front" (bool, default false); also the dock's pin button.
pub(crate) const DOCK_KEEP_IN_FRONT: &str = "dock_keep_in_front";
/// General > Multitasking: the dock opens by itself while no Webox window is in front (bool, default false).
pub(crate) const DOCK_AUTO_COVERED: &str = "dock_auto_covered";
/// Unix ms of the last look at the "Calling" tab: missed calls after it make the rail badge (calls_page).
pub(crate) const CALLS_SEEN_MS: &str = "calls_seen_ms";

/// One-time fixes of stored settings, run on load. Returns true when something changed (then the file is written back).
/// * Before 0.3.6 the settings pages wrote `dock_stay_on_top` / `multitasking`, which the dock never read: they move to
///   [`DOCK_KEEP_IN_FRONT`] / [`DOCK_AUTO_COVERED`] (a value the dock itself already wrote wins).
/// * [`CALLS_SEEN_MS`] starts at `now_ms`, so a first start does not badge every missed call of the last ten days.
pub(crate) fn migrate(s: &mut serde_json::Value, now_ms: i64) -> bool {
    let Some(o) = s.as_object_mut() else { return false };
    let mut changed = false;
    for (old, new) in [("dock_stay_on_top", DOCK_KEEP_IN_FRONT), ("multitasking", DOCK_AUTO_COVERED)] {
        if let Some(v) = o.remove(old) {
            if !o.contains_key(new) { o.insert(new.into(), v); }
            changed = true;
        }
    }
    if !o.get(CALLS_SEEN_MS).is_some_and(|v| v.is_i64()) { o.insert(CALLS_SEEN_MS.into(), json!(now_ms)); changed = true; }
    changed
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    #[test]
    fn dock_keys_written_by_the_old_settings_pages_move_to_the_keys_the_dock_reads() {
        let mut s = json!({"dock_stay_on_top": true, "multitasking": true, "calls_seen_ms": 5});
        assert!(migrate(&mut s, 9));
        assert_eq!(s, json!({"dock_keep_in_front": true, "dock_auto_covered": true, "calls_seen_ms": 5}));
        assert!(!migrate(&mut s, 9), "runs once");
    }

    #[test]
    fn a_value_the_dock_already_wrote_wins_over_the_old_key() {
        let mut s = json!({"dock_stay_on_top": true, "dock_keep_in_front": false, "calls_seen_ms": 5});
        assert!(migrate(&mut s, 9));
        assert_eq!(s, json!({"dock_keep_in_front": false, "calls_seen_ms": 5}));
    }

    #[test]
    fn missed_calls_from_before_the_first_start_are_not_badged() {
        let mut s = json!({});
        assert!(migrate(&mut s, 1234));
        assert_eq!(s[CALLS_SEEN_MS], json!(1234));
    }
}

