#![allow(clippy::too_many_arguments)]
use crate::*;

/// Right inset of list rows from the scroll area's right edge (pt).
const ROW_INSET_L: f32 = 12.0;
const ROW_INSET_R: f32 = 4.0;

/// Header height (pt; 44.8 in Standard density). The 28 pt search field and the 32 pt avatar are centred at 18.4 pt.
pub(crate) fn header_h() -> f32 { crate::ui::theme::density::m().header_h }
/// Collapsed rail width (pt); the selected 40 pt circle spans x 4.8-44.8 (centre 24.8), vertical centre 72.4.
pub(crate) const RAIL_NARROW: f32 = 60.8;
/// Margin around all content while the window is not maximised (pt): the window background shows through, it is the resize
/// border. Every edge offset of the maximised layout grows by this much.
pub(crate) const EDGE_INSET: f32 = 6.0;
pub(crate) use header_layout::MIN_WINDOW;

/// The current content inset: [`EDGE_INSET`] while the window is neither maximised nor fullscreen, else 0. Divided by the app
/// zoom so the grab border keeps its on-screen size.
pub(crate) fn edge_inset(ctx: &egui::Context) -> f32 {
    let (max, full) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if max || full { 0.0 } else { EDGE_INSET / ctx.zoom_factor().max(0.1) }
}

/// Running on a Wayland session (no window positions, no always-on-top, no `set_visible`).
pub(crate) fn wayland() -> bool {
    #[cfg(target_os = "linux")]
    { crate::platform::linux::session() == crate::platform::linux::Session::Wayland }
    #[cfg(not(target_os = "linux"))]
    { false }
}

/// After `StartDrag` / `BeginResize` the Wayland compositor eats the button release, so egui keeps `any_down` set until the next
/// press. Call when sending either command.
pub(crate) fn note_os_grab(ctx: &egui::Context) { if wayland() { ctx.data_mut(|d| d.insert_temp(egui::Id::new("os-grab"), true)); } }

/// `pointer.any_down()`, but false after [`note_os_grab`] until a fresh button press arrives.
pub(crate) fn pointer_down(ctx: &egui::Context) -> bool {
    let id = egui::Id::new("os-grab");
    if ctx.input(|i| i.raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. }))) { ctx.data_mut(|d| d.remove::<bool>(id)); }
    ctx.input(|i| i.pointer.any_down()) && !ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false)
}

/// One resize strip over `r` (`side`: 0 top, 1 bottom, 2 left, 3 right): cursor and `BeginResize`; within 12 on-screen pt of a
/// window corner it resizes diagonally.
fn edge_zone(ui: &mut egui::Ui, r: Rect, id: &str, side: u8) {
    use egui::ResizeDirection as D;
    let resp = ui.interact(r, egui::Id::new(("edge-strip", id)), Sense::drag());
    let screen = ui.ctx().screen_rect();
    let c = 12.0 / ui.ctx().zoom_factor().max(0.1);
    let dir = resp.hover_pos().or(resp.interact_pointer_pos()).map(|p| {
        let (l, rt, t, b) = (p.x < screen.left() + c, p.x > screen.right() - c, p.y < screen.top() + c, p.y > screen.bottom() - c);
        match (side, l, rt, t, b) {
            (_, true, _, true, _) => D::NorthWest, (_, _, true, true, _) => D::NorthEast, (_, true, _, _, true) => D::SouthWest, (_, _, true, _, true) => D::SouthEast,
            (0, ..) => D::North, (1, ..) => D::South, (2, ..) => D::West, _ => D::East,
        }
    });
    if let Some(d) = dir {
        ui.ctx().set_cursor_icon(match d { D::North | D::South => egui::CursorIcon::ResizeVertical, D::West | D::East => egui::CursorIcon::ResizeHorizontal,
            D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe, _ => egui::CursorIcon::ResizeNeSw });
        if resp.drag_started() { note_os_grab(ui.ctx()); ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(d)); }
    }
}

/// The inset strips (call before every other panel): window background, and dragging them resizes the window; within 12 pt
/// of a corner they resize diagonally.
pub(crate) fn edge_strips(ctx: &egui::Context) {
    let e = edge_inset(ctx);
    if e <= 0.0 { return; }
    let f = egui::Frame::NONE;
    egui::TopBottomPanel::top("edge-top").exact_height(e).show_separator_line(false).frame(f).show(ctx, |ui| edge_zone(ui, ui.max_rect(), "top", 0));
    egui::TopBottomPanel::bottom("edge-bottom").exact_height(e).show_separator_line(false).frame(f).show(ctx, |ui| edge_zone(ui, ui.max_rect(), "bottom", 1));
    egui::SidePanel::left("edge-left").exact_width(e).resizable(false).show_separator_line(false).frame(f).show(ctx, |ui| edge_zone(ui, ui.max_rect(), "left", 2));
    egui::SidePanel::right("edge-right").exact_width(e).resizable(false).show_separator_line(false).frame(f).show(ctx, |ui| edge_zone(ui, ui.max_rect(), "right", 3));
}

/// Resize strips laid over the content of a frameless secondary viewport (call window, dock, share picker; they take no layout
/// space): Linux has no server-side borders to grab. Not while maximised / fullscreen; callers skip fixed-size windows.
pub(crate) fn edge_overlay(ctx: &egui::Context) {
    #[cfg(target_os = "linux")]
    {
        let e = edge_inset(ctx);
        if e <= 0.0 { return; }
        let s = ctx.screen_rect();
        egui::Area::new(egui::Id::new("edge-overlay")).order(egui::Order::Foreground).fixed_pos(s.min).interactable(false).show(ctx, |ui| {
            ui.set_clip_rect(s);
            edge_zone(ui, Rect::from_min_max(s.min, pos2(s.right(), s.top() + e)), "top", 0);
            edge_zone(ui, Rect::from_min_max(pos2(s.left(), s.bottom() - e), s.max), "bottom", 1);
            edge_zone(ui, Rect::from_min_max(s.min, pos2(s.left() + e, s.bottom())), "left", 2);
            edge_zone(ui, Rect::from_min_max(pos2(s.right() - e, s.top()), s.max), "right", 3);
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = ctx;
}
const HEADER_CY: f32 = 18.4;

/// Window control `i` (1 = close, 2 = maximise, 3 = minimise): its click / hover cell and glyph centre. `header` is the header
/// panel (inset by the resize strips when not maximised), `win` the whole window. Glyphs 14 pt from the header's right edge,
/// 40.2 pt apart; 40 pt cells over the header's height, stretched up to the window's top edge, the close cell also to its
/// right edge.
fn control_cell(header: Rect, win: Rect, i: f32) -> (Rect, egui::Pos2) {
    let c = pos2(header.right() - 14.0 - 40.2 * (i - 1.0), header.top() + 18.8);
    let mut r = Rect::from_center_size(pos2(c.x, header.center().y), Vec2::new(40.0, header.height())).intersect(header);
    r.min.y = win.top();
    if i == 1.0 { r.max.x = win.right(); }
    (r, c)
}

/// The list row's draft icon: shown when the row has a draft and is not the selected conversation. An unread row keeps
/// its unread mark instead. A whitespace-only text counts as no draft (like the disabled send button).
pub(crate) fn shows_draft_icon(draft: &str, selected: bool, unread: bool) -> bool {
    !selected && !unread && !draft.trim().is_empty()
}

#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn draft_icon_rule() {
        assert!(shows_draft_icon("Hello", false, false));
        assert!(!shows_draft_icon("Hello", true, false)); // the open conversation never shows it
        assert!(!shows_draft_icon("Hello", false, true)); // unread wins
        assert!(!shows_draft_icon("  \n", false, false));
        assert!(!shows_draft_icon("", false, false));
    }
}

#[cfg(test)]
mod control_cell_tests {
    use super::*;
    #[test]
    fn control_cells_reach_the_window_edge() {
        // Not maximised: 6 pt strips around a 1000 x 700 window.
        let win = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1000.0, 700.0));
        let header = Rect::from_min_max(pos2(6.0, 6.0), pos2(994.0, 6.0 + header_h()));
        let (close, cg) = control_cell(header, win, 1.0);
        assert_eq!((close.top(), close.right(), close.bottom()), (0.0, 1000.0, header.bottom()));
        assert!(close.contains(pos2(999.5, 0.5))); // the corner pixel closes
        assert_eq!(cg, pos2(980.0, 24.8)); // glyph 20 pt from the window edge, unchanged
        let (max, mg) = control_cell(header, win, 2.0);
        assert_eq!(max.top(), 0.0);
        assert!((close.left() - max.right()).abs() < 0.25); // 40 pt cells on the 40.2 pt glyph pitch
        assert_eq!(mg.x, 1000.0 - 6.0 - 14.0 - 40.2);
        // Maximised: no strips, the cells end at the window edge.
        let (close, _) = control_cell(win.with_max_y(header_h()), win, 1.0);
        assert_eq!((close.top(), close.right()), (0.0, 1000.0));
    }
}

/// Single-line text truncated with an ellipsis, vertically centred on `y`.
pub(crate) fn fit_text(p: &egui::Painter, x: f32, y: f32, text: &str, font: FontId, color: Color32, max_w: f32) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
    let g = p.layout_job(job);
    let pos = pos2(x, y - g.size().y / 2.0);
    let r = Rect::from_min_size(pos, g.size());
    p.galley(pos, g, color);
    r
}

/// Header ghost-button background (Momentum button-ghost, dark theme: transparent, hovered gray-70 #545454, pressed
/// gray-80 #3B3B3B), fully rounded; pointer cursor on hover.
/// `force` pins the hover look for --screenshot checks (WEBOX_SHOT_HOVER).
fn header_chip_fill(ui: &egui::Ui, t: &Th, resp: &egui::Response, rect: Rect, force: bool) {
    if resp.hovered() || resp.is_pointer_button_down_on() || force {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        // globalHeader-buttonStatus-background / -buttonIcon-background / -buttonDevice-background: hover white 7 %, pressed 20 %
        // (black 7 % / 20 % in light themes, where the header is light).
        let c = if resp.is_pointer_button_down_on() { t.at(tok!("globalHeader-buttonStatus-background:pressed")) } else { t.at(tok!("globalHeader-buttonStatus-background:hovered")) };
        ui.painter().rect_filled(rect, rect.height() / 2.0, c);
    }
}

/// Now in Unix milliseconds (for "Active 3 hours ago").
pub(crate) fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

impl App {
    /// The profile picture for a person known by id and/or email (the keys the cloud thread files them under).
    pub(crate) fn photo_for(&mut self, ctx: &egui::Context, id: &str, email: &str) -> Option<egui::TextureId> {
        [id, email].into_iter().filter(|k| !k.is_empty()).find_map(|k| self.photo(ctx, k))
    }

    /// The other person of a direct space: from the people list by name, else from the space's participants.
    pub(crate) fn direct_person(&self, s: &Space) -> Option<Person> {
        self.people.iter().find(|p| p.name == s.title).cloned()
            .or_else(|| s.participants.iter().find(|p| p.name == s.title).cloned())
            .or_else(|| s.participants.iter().find(|p| p.email != self.me.email && p.id != self.me.id).cloned())
    }

    /// The line under a person's name: "Available", "Active 3 hours ago", "Do not disturb", empty if unknown.
    pub(crate) fn person_line(&self, p: &Person, now_ms: i64) -> String {
        // The whole line (DND end, out of office, custom text, "Active 3 hours ago") comes from presence.rs.
        self.presence_line(p, now_ms)
    }

    pub(crate) fn top_bar(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        header_layout::keep_min_size(ctx);
        egui::TopBottomPanel::top("top").exact_height(header_h()).show_separator_line(false).frame(egui::Frame::default()).show(ctx, |ui| {
            let full = ui.max_rect();
            let cy = full.top() + HEADER_CY;
            let forced = std::env::var("WEBOX_SHOT_HOVER").unwrap_or_default();
            // Drag / double-click on the empty header area: a custom title bar.
            let drag = ui.interact(full, ui.id().with("titlebar"), Sense::click_and_drag());
            if drag.drag_started() { note_os_grab(ctx); ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
            if drag.double_clicked() {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            // Own avatar (40) with presence badge, then "Set a status".
            let (me, st) = (self.me.name.clone(), self.me.status.clone());
            let label = if self.client.is_some() || self.status.starts_with("Offline (sample") { self.status_label(ctx) } else { self.status.clone() };
            let lw = ui.fonts(|f| f.layout_no_wrap(label.clone(), F::Body.semibold(), t.text1).size().x);
            let dev_txt = tr("Connect to a device");
            let gw = ui.fonts(|f| f.layout_no_wrap(dev_txt.to_string(), F::Body.semibold(), t.text1).size().x);
            // Shared by all header items: narrow windows elide the status and device labels instead of overlapping.
            let hl = header_layout::header_layout(full, cy, lw, gw);
            // Header avatar: 32 pt, photo 29 pt inside a thin light ring, presence dot 11.6 pt centred 0.356 / 0.28 of the diameter
            // right / below the centre; avatar centre 28 pt from the window's left edge.
            let ar = hl.avatar;
            let mut ac = ui.new_child(egui::UiBuilder::new().max_rect(ar));
            let photo = [self.me.email.clone(), self.me.id.clone()].iter().find_map(|k| self.photo(ctx, k));
            // The badge's 1.2 pt ring is cut out of the photo (the header shows through), the dot drawn on the device-pixel grid.
            super::widgets::avatar_photo_with(&mut ac, &mut self.icons, t, &me, 32.0, false, None, Some(&st), photo, super::widgets::BadgeGeom::HEADER);
            // "Set a status": a pill button (hover #1C1C1C / pressed #121212 = appHeader-buttonStatus, dark theme): 28 high,
            // radius 14, text padding 14 / 14, 14 px / 600, 16 after the avatar, 1 px border; the text elides when narrow.
            if let Some(chip) = hl.status {
                let cresp = ui.interact(chip, egui::Id::new("status-chip"), Sense::click());
                header_chip_fill(ui, t, &cresp, chip, forced == "status");
                let shown = fit_text(ui.painter(), chip.left() + 15.0, cy, &label, F::Body.semibold(), t.at(tok!("globalHeader-buttonStatus-text")), chip.width() - 30.0);
                let cresp = if shown.width() + 0.5 < lw { cresp.on_hover_text(&label) } else { cresp };
                if cresp.clicked() { let cs = self.custom_status(ctx); super::profile_menu::open_status_dialog(ctx, &cs); }
            }
            // The two do different things: the avatar opens the profile menu ("Profile and settings"), the status button opens the
            // custom-status dialog directly. Avatar tooltip from the pack (QObject).
            let aresp = ui.interact(ar, egui::Id::new("avatar-btn"), Sense::click()).on_hover_text(tr("Click your avatar to set your status, change notification settings and download the mobile app."));
            if aresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            if aresp.clicked() { super::menu::menu_toggle(ctx, "status"); }
            self.profile_menu(ctx, t, ar, now);

            // Centre group: back / forward, search pill (fixed 354 x 28), +.
            let sr = hl.search;
            let can_back = self.hpos > 0;
            let can_fwd = self.hpos + 1 < self.history.len();
            let chev = |s: &mut Self, ui: &mut egui::Ui, id: &str, icon: &'static str, rect: Rect, on: bool| {
                let resp = ui.interact(rect, egui::Id::new(id), Sense::click());
                if on { header_chip_fill(ui, t, &resp, Rect::from_center_size(rect.center(), Vec2::splat(28.0)), false); }
                // globalHeader-buttonIcon-icon / :disabled.
                s.icons.paint(ui, icon, Rect::from_center_size(rect.center(), Vec2::splat(14.0)), if on { t.at(tok!("globalHeader-buttonIcon-icon")) } else { t.at(tok!("globalHeader-buttonIcon-icon:disabled")) });
                resp
            };
            if chev(self, ui, "nav-back", "arrow-left-optical_10", hl.back, can_back).on_hover_text(tr("Back")).clicked() { self.act(Act::Back, ctx) }
            if chev(self, ui, "nav-fwd", "arrow-right-optical_10", hl.fwd, can_fwd).on_hover_text(tr("Forward")).clicked() { self.act(Act::Forward, ctx) }
            // globalHeader-SearchEditField-background: #3B3B3B / #DEDEDE, hovered #545454 / #C9C9C9, pressed = checked; outline
            // appHeader-searchBar-outline-active #3B3B3B.
            // Settings > Customize the interface can hide the search field, +, the device button and the AI mark (Ctrl+F still searches).
            use crate::ui::customize::HEADER as HG;
            if self.ui_shown(&HG, "search") {
                let sfill = if ui.rect_contains_pointer(sr) { if ui.input(|i| i.pointer.primary_down()) { t.at(tok!("globalHeader-SearchEditField-background:checked")) } else { t.at(tok!("globalHeader-SearchEditField-background:hovered")) } } else { t.search };
                ui.painter().rect_filled(sr, 14.0, sfill);
                ui.painter().rect_stroke(sr, 14.0, Stroke::new(1.0_f32, t.at(tok!("appHeader-searchBar-outline-active"))), egui::StrokeKind::Inside);
                self.icons.paint(ui, "search-regular", Rect::from_center_size(sr.left_center() + Vec2::new(24.0, 0.0), Vec2::splat(16.0)), t.at(tok!("globalHeader-textfield-searchIcon")));
                // The shortcut hint only while the field has its full width (a zoomed UI below the minimum window shrinks it).
                let hint = if sr.width() >= header_layout::SEARCH_W { ui.painter().text(sr.right_center() - Vec2::new(11.0, 0.0), egui::Align2::RIGHT_CENTER, crate::shortcuts::native_hint(&tr("Ctrl+F")), F::Body.font(), t.text2).width() + 19.0 } else { 12.0 };
                // The pill is a button; a click / Ctrl+F opens the global search popup (ui/global_search.rs).
                fit_text(ui.painter(), sr.left() + 40.0, cy, tr("Search, meet, and call"), F::BodyLarge.font(), t.text2, sr.width() - 40.0 - hint);
                let sresp = ui.interact(sr, egui::Id::new("search-pill"), Sense::click());
                if sresp.clicked() { super::global_search::toggle(ctx); }
            }
            super::global_search::set_pill(ctx, sr); // the popup's anchor, also while the field is hidden
            if std::mem::take(&mut self.focus_search) { super::global_search::open(ctx); }
            // The + circle (26 px, light outline).
            let pr = hl.plus;
            if self.ui_shown(&HG, "plus") {
                let presp = ui.interact(pr, egui::Id::new("top-plus"), Sense::click());
                // globalHeader-buttonIcon-background: hovered 7 %, pressed 20 %. Ring #CFCFCF dark, #454545 light.
                if presp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); ui.painter().circle_filled(pr.center(), 13.0, if presp.is_pointer_button_down_on() { t.at(tok!("globalHeader-buttonIcon-background:pressed")) } else { t.at(tok!("globalHeader-buttonIcon-background:hovered")) }); }
                ui.painter().circle_stroke(pr.center(), 12.4, Stroke::new(1.2_f32, t.c(C::HeaderAvatarRing)));
                self.icons.paint(ui, "plus-regular", Rect::from_center_size(pr.center(), Vec2::splat(14.0)), t.at(tok!("globalHeader-buttonIcon-icon")));
                // The + opens the global menu (ui/header_plus.rs); tooltip "Start a conversation".
                let plus_open = super::menu::menu_is_open(ctx, super::header_plus::KEY);
                let presp = if plus_open { presp } else { presp.on_hover_text(crate::trq!("Start a conversation")) };
                if presp.clicked() { super::menu::menu_toggle(ctx, super::header_plus::KEY) }
            }
            self.header_plus_menu(ctx, t, pr, now);

            // Window controls: glyph centres 14.0 / 54.4 / 94.4 pt from the right edge (40 pt pitch), close X 8.8 pt centred at y 18.8,
            // maximise square 11.2 pt at y 17.6, minimise line 11.2 pt at y 18.8; 40 pt cells centred on the glyphs, cut at the window
            // edge. Hover colours: globalHeader-maxMinButton-background (7 %), globalHeader-closeButton-background (#AB0A15, white glyph
            // while hovered).
            // Not maximised, the window has the 6 pt resize strips (edge_strips) around the header: the cells reach over them to
            // the window's outer edge (all three to the top, the close button also to the right, so the corner pixel closes the
            // window as on Windows). They live in an Area above the strips (Background panels), so those clicks don't resize.
            let win = ctx.screen_rect();
            let wc = |i: f32| control_cell(full, win, i);
            let btn = |_ui: &mut egui::Ui, id: &str, (rect, c): (Rect, egui::Pos2), hover: Color32, glyph: &dyn Fn(&egui::Painter, egui::Pos2, bool)| {
                egui::Area::new(egui::Id::new(id)).order(egui::Order::Middle).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
                    let (r, resp) = ui.allocate_exact_size(rect.size(), Sense::click());
                    if resp.hovered() { ui.painter().rect_filled(r, 0.0, hover); }
                    glyph(ui.painter(), c, resp.hovered());
                    resp.clicked()
                }).inner
            };
            let (mm_hover, mm_glyph) = (t.at(tok!("globalHeader-maxMinButton-background:hovered")), t.at(tok!("globalHeader-maxMinButton-text")));
            if btn(ui, "win-close", wc(1.0), t.at(tok!("globalHeader-closeButton-background:hovered")), &|p, c, hov| {
                let d = 4.4;
                let g = if hov { t.at(tok!("globalHeader-closeButton-text:hovered")) } else { t.at(tok!("globalHeader-closeButton-text")) };
                p.line_segment([c - Vec2::new(d, d), c + Vec2::new(d, d)], Stroke::new(1.0_f32, g));
                p.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], Stroke::new(1.0_f32, g));
            }) { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            if btn(ui, "win-max", wc(2.0), mm_hover, &|p, c, _| {
                p.rect_stroke(Rect::from_center_size(c - Vec2::new(0.0, 1.2), Vec2::splat(11.2)), 1.0, Stroke::new(1.0_f32, mm_glyph), egui::StrokeKind::Inside);
            }) {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            if btn(ui, "win-min", wc(3.0), mm_hover, &|p, c, _| {
                p.line_segment([c - Vec2::new(5.6, 0.0), c + Vec2::new(5.6, 0.0)], Stroke::new(1.0_f32, mm_glyph));
            }) { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }

            // "Connect to a device" with its device icon (label elided when narrow, icon only at the narrowest, the label
            // then as tooltip), then the AI assistant.
            let tr_rect = hl.device;
            if self.ui_shown(&HG, "device") {
                let resp = ui.interact(tr_rect, egui::Id::new("connect-device"), Sense::click());
                header_chip_fill(ui, t, &resp, tr_rect, forced == "device");
                let dev_ink = t.at(tok!("globalHeader-buttonDevice-text-inactive"));
                let (icon_c, full_label) = if hl.device_label {
                    let shown = fit_text(ui.painter(), tr_rect.left() + 32.0, cy, dev_txt, F::Body.semibold(), dev_ink, tr_rect.width() - 44.0);
                    (pos2(tr_rect.left() + 20.0, cy), shown.width() + 0.5 >= gw)
                } else { (tr_rect.center(), false) };
                self.icons.paint(ui, "device-connection-regular", Rect::from_center_size(icon_c, Vec2::splat(16.0)), dev_ink);
                let resp = if full_label { resp } else { resp.on_hover_text(dev_txt) };
                if resp.clicked() { super::menu::menu_toggle(ctx, super::app_dialogs::DEVICE) } // device list (ui/app_dialogs.rs)
            }
            // The Webex AI assistant mark (cisco-ai-assistant-color-gradient.svg), 18 pt, its own colours.
            if !self.ui_shown(&HG, "ai") { return; }
            let ai = hl.ai;
            let air = ui.interact(ai, egui::Id::new("ai-assistant"), Sense::click());
            header_chip_fill(ui, t, &air, ai, forced == "ai");
            self.icons.art(ui, "cisco-ai-assistant-color-gradient", Rect::from_center_size(ai.center(), Vec2::splat(18.0)));
            if air.on_hover_text(tr("AI Assistant")).clicked() { self.toast(tr("The AI Assistant is not available in this version"), now) }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn row(&mut self, ui: &mut egui::Ui, t: &Th, title: &str, sub: &str, sub_color: Color32, selected: bool, unread: bool, space: bool, av_bg: Option<Color32>, status: Option<&str>, photo: Option<egui::TextureId>) -> egui::Response {
        // Row geometry from the density table, or the compact view (Standard: 50 pt rows on a 51 pt pitch), inset from the list's edges.
        let g = crate::ui::theme::density::ListRow::current(self.compact_view());
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), g.pitch), Sense::hover());
        let r = Rect::from_min_max(full.min + Vec2::new(ROW_INSET_L, 0.0), pos2(full.max.x - ROW_INSET_R, full.min.y + g.h));
        let resp = ui.interact(r, ui.id().with(("row", title, sub, self.row_salt.clone())), Sense::click());
        let rad = g.h / 2.0;
        if selected { ui.painter().rect_filled(r, rad, t.selected); if crate::ui::theme::decor::on(collabhost_plugin_api::deco::SELECTED_ROW) { crate::ui::theme::decor::outline(ui.painter(), r, rad, 1.5, &crate::ui::theme::decor::stripes(), 0.0); } } else if resp.is_pointer_button_down_on() { ui.painter().rect_filled(r, rad, t.at(tok!("listitem-background:pressed"))); } else if resp.hovered() { ui.painter().rect_filled(r, rad, t.at(tok!("listitem-background:hovered"))); } // listitem-background: 7 % hover, 11 % pressed
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(pos2(r.left() + (g.avatar_left + g.avatar / 2.0), r.center().y), Vec2::splat(g.avatar))));
        avatar_photo(&mut c, &mut self.icons, t, title, g.avatar, space, av_bg, status, photo);
        let tx = r.left() + g.text_x;
        let hovered_row = selected || ui.rect_contains_pointer(r);
        // Draft icon ("draft-indicator-bold", 14 x 14, right margin 14; the "···" button, 24 wide with right margin 14, appears to
        // its right on hover), colour wx-brickletNotificationIcon-background.
        let draft_at = self.row_draft.then(|| pos2(r.right() - if hovered_row { 14.0 + 24.0 + 14.0 } else { 14.0 } - 7.0, r.center().y));
        let max_w = draft_at.map_or(r.right() - 34.0, |c| c.x - 7.0 - 6.0) - tx; // the title's right padding 6
        let tf = if unread { semibold(g.title_pt) } else { FontId::proportional(g.title_pt) };
        // Like Webex: read conversations' names in the secondary grey, the selected or an unread one in the primary white.
        let title_col = if selected || unread { t.text1 } else { t.text2 };
        if g.one_line {
            compact_list::one_line(ui.painter(), tx, r.center().y, max_w, title, tf, title_col, sub, sub_color, t.text2);
        } else if sub.is_empty() {
            fit_text(ui.painter(), tx, r.center().y, title, tf, title_col, max_w);
        } else {
            fit_text(ui.painter(), tx, r.center().y - 9.5, title, tf, title_col, max_w);
            fit_text(ui.painter(), tx, r.center().y + 10.0, &sub.replace('\n', " "), F::Secondary.font(), sub_color, max_w);
        }
        if let Some(c) = draft_at {
            let col = if super::menu::light(t) { hex(0x000000) } else { hex(0x949494) }; // gray-50 / black-100
            self.icons.paint(ui, "draft-indicator-bold", Rect::from_center_size(c, Vec2::splat(14.0)), col);
        }
        if hovered_row {
            let dots = Rect::from_center_size(r.right_center() - Vec2::new(26.0, 0.0), Vec2::splat(24.0));
            // The "···" button opens the row menu (its own hit area on top of the row).
            let dresp = if self.row_salt.is_empty() { None } else { Some(ui.interact(dots, ui.id().with(("row-dots", &self.row_salt)), Sense::click())) };
            if let Some(d) = &dresp {
                if d.hovered() { ui.painter().circle_filled(dots.center(), 12.0, t.ink_a(0.12)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                if d.clicked() {
                    let ctx = ui.ctx().clone();
                    let same = ctx.data(|dd| dd.get_temp::<(String, Rect)>(egui::Id::new("row-menu-target"))).is_some_and(|(s, _)| s == self.row_salt) && super::menu::menu_is_open(&ctx, "row");
                    if same { super::menu::menu_close(&ctx, "row"); } else {
                        ctx.data_mut(|dd| dd.insert_temp(egui::Id::new("row-menu-target"), (self.row_salt.clone(), dots)));
                        super::menu::menu_open(&ctx, "row");
                    }
                }
            }
            self.icons.paint(ui, "more-regular", Rect::from_center_size(r.right_center() - Vec2::new(26.0, 0.0), Vec2::splat(16.0)), t.text2)
        }
        else if unread { let c = if crate::ui::theme::decor::on(collabhost_plugin_api::deco::BADGE_CYCLE) { crate::ui::theme::decor::cycle((r.top() / r.height().max(1.0)).round() as usize).unwrap_or(t.unread) } else { t.unread }; ui.painter().circle_filled(r.right_center() - Vec2::new(20.0, 0.0), 5.0, c); }
        resp
    }

    pub(crate) fn section(&mut self, ui: &mut egui::Ui, t: &Th, key: &'static str, label: &str, star: bool) -> bool {
        let open = !self.collapsed.contains(key);
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), crate::ui::theme::density::m().section_h), Sense::hover());
        let r = Rect::from_min_max(full.min + Vec2::new(ROW_INSET_L, 0.0), full.max);
        let resp = ui.interact(r, ui.id().with(("section", key)), Sense::click());
        // The chevron glyph is 12.8 x 6.4 pt centred at x 32, 0.8 pt below the label's centre.
        let icon_r = Rect::from_center_size(r.left_center() + Vec2::new(12.0, 2.0), Vec2::splat(18.0));
        self.icons.paint(ui, if open { "arrow-down-regular" } else { "arrow-right-regular" }, icon_r, t.at(tok!("listitem-icon"))); // listitem-icon
        let g = ui.painter().text(r.left_center() + Vec2::new(28.0, 0.0), egui::Align2::LEFT_CENTER, tr(label), F::Body.font(), t.text2);
        if star { self.icons.paint(ui, "favorite-filled", Rect::from_center_size(pos2(g.right() + 9.0, r.center().y), Vec2::splat(14.0)), t.star) }
        if resp.clicked() { if open { self.collapsed.insert(key); } else { self.collapsed.remove(key); } }
        open
    }

    /// `space_row` for `self.spaces[i]`; a row outside the list's visible area only takes its place (rows have a fixed
    /// 51 pt pitch, see `row`), so a long list costs what its visible part costs.
    pub(crate) fn space_row_at(&mut self, ui: &mut egui::Ui, t: &Th, i: usize) {
        let pitch = crate::ui::theme::density::ListRow::current(self.compact_view()).pitch;
        let place = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), pitch));
        if !ui.is_rect_visible(place) { ui.allocate_exact_size(place.size(), Sense::hover()); return; }
        let Some(s) = self.spaces.get(i).cloned() else { return };
        self.space_row(ui, t, &s);
    }

    pub(crate) fn space_row(&mut self, ui: &mut egui::Ui, t: &Th, s: &Space) {
        let person = if s.direct { self.direct_person(s) } else { None };
        let now_ms = now_ms();
        let team = self.teams.iter().find(|x| x.id == s.team).map(|x| x.name.clone());
        let tcol = self.teams.iter().find(|x| x.id == s.team).and_then(|x| x.color);
        let (sub, col) = match (&person, &team) {
            (Some(p), _) => (self.person_line(p, now_ms), t.text2),
            (None, Some(name)) => (name.clone(), team_tones(t, tcol, &s.team).1),
            _ if s.public => (tr("Public").to_string(), t.at(tok!("modal-decorativeTitle-slate"))),
            _ => (String::new(), t.text2),
        };
        let av = team.as_ref().map(|_| team_tones(t, tcol, &s.team).0);
        let sel = self.selected.as_ref() == Some(&s.id);
        let status = person.as_ref().map(|p| p.status.clone());
        // Direct: the person's photo. Group: the space's own picture (e.g. a company logo) when it has one, else the letter.
        let photo = if s.direct { person.as_ref().and_then(|p| self.photo_for(ui.ctx(), &p.id, &p.email)) } else { self.space_avatar(ui.ctx(), &s.id).map(|(tid, _)| tid) };
        self.row_salt = s.id.clone(); // rows with equal titles must still get distinct widget ids
        self.row_draft = shows_draft_icon(self.drafts.get(&s.id).map_or("", |d| d.as_str()), sel, s.unread);
        let resp = self.row(ui, t, &s.title, &sub, col, sel, s.unread, !s.direct, av, status.as_deref(), photo);
        self.row_draft = false;
        if resp.clicked() { self.open_space(s.id.clone()) }
        // Right-click opens the row menu at the pointer (see space_menu.rs).
        if resp.secondary_clicked() {
            let at = Rect::from_min_size(ui.ctx().input(|i| i.pointer.interact_pos()).unwrap_or(resp.rect.center()), Vec2::splat(1.0));
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("row-menu-target"), (s.id.clone(), at)));
            super::menu::menu_open(ui.ctx(), "row");
        }
        self.row_salt.clear();
    }
}

pub(crate) mod compact_list;
mod header_layout;
mod rail;
