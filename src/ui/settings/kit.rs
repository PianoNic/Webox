//! Settings controls (check box, radio button, combo box, slider, pill button, link) and the page building blocks (section,
//! page frame), in the Webex desktop look.
//!
//! Metrics: checkbox box 16, radius 2, indicator left margin 8, label left padding 8 (text starts at 32);
//! radio 16 (ring 1 + 4 px, dot 6), label padding 8; combo box height 32, radius 8, left padding 12, arrow-down-bold 12,
//! popup y+4, radius 12, padding 8, rows radius 6, check-bold 16; slider track 2 (radius 2), handle 16; section header
//! in the header font, description in the secondary font, section top margin 16, spacing 4.
//! Colours: the core tokens (inputBackground, inputOutline, input-focusRing, listItem-primary, button-secondary,
//! wx-default-border; MomentumDark / MomentumDefault resolved) in [`Tok`], the component tokens (checkbox-*, radio-*,
//! combobox-*, slider-*, menulistbackground-*) in [`Ctl`].
use crate::*;

pub(crate) fn is_dark(t: &Th) -> bool { !t.light }

pub(crate) struct Tok { pub input_bg: Color32, pub outline: Color32, pub outline_hover: Color32, pub focus: Color32, pub item_hover: Color32, pub item_pressed: Color32,
    pub btn_hover: Color32, pub btn_press: Color32, pub border: Color32, pub popup_bg: Color32 }
pub(crate) fn tok(t: &Th) -> Tok {
    Tok { input_bg: t.at(tok!("inputBackground")), outline: t.at(tok!("inputOutline")), outline_hover: t.at(tok!("inputOutline:hovered")), focus: t.at(tok!("input-focusRing")),
        item_hover: t.at(tok!("listItem-primary:hovered")), item_pressed: t.at(tok!("listItem-primary:pressed")), btn_hover: t.at(tok!("button-secondary:hovered")),
        btn_press: t.at(tok!("button-secondary:pressed")), border: t.at(tok!("wx-default-border")),
        popup_bg: t.at(tok!("wx-menuList-background")) /* the settings search popup */ }
}

/// Three-state colour (normal / hovered / pressed) of a component token.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct St { pub normal: Color32, pub hovered: Color32, pub pressed: Color32 }
impl St {
    fn of(n: Color32, h: Color32, p: Color32) -> St { St { normal: n, hovered: h, pressed: p } }
    pub(crate) fn pick(&self, r: &egui::Response) -> Color32 { if r.is_pointer_button_down_on() { self.pressed } else if r.hovered() { self.hovered } else { self.normal } }
}

/// Component tokens of the MomentumDark / MomentumDefault themes.
pub(crate) struct Ctl {
    /// checkbox-checked-background = radio-checked-background.
    pub checked_bg: St,
    /// checkbox-checked-icon (the tick) = radio-checked-icon (the dot).
    pub checked_icon: Color32,
    /// checkbox-unchecked-background = radio-unchecked-background.
    pub unchecked_bg: St,
    /// checkbox-unchecked-border = radio-unchecked-border (same in every state).
    pub unchecked_border: Color32,
    pub track_empty: Color32,
    pub track_indicator: Color32,
    pub handle_bg: St,
    pub handle_border: Color32,
}
pub(crate) fn ctl(t: &Th) -> Ctl {
    Ctl { checked_bg: St::of(t.at(tok!("checkbox-checked-background")), t.at(tok!("checkbox-checked-background:hovered")), t.at(tok!("checkbox-checked-background:pressed"))),
        checked_icon: t.at(tok!("checkbox-checked-icon")),
        unchecked_bg: St::of(t.at(tok!("checkbox-unchecked-background")), t.at(tok!("checkbox-unchecked-background:hovered")), t.at(tok!("checkbox-unchecked-background:pressed"))),
        unchecked_border: t.at(tok!("checkbox-unchecked-border")),
        track_empty: t.at(tok!("slider-track-empty-background")), track_indicator: t.at(tok!("slider-track-indicator-background")),
        handle_bg: St::of(t.at(tok!("slider-handle-background")), t.at(tok!("slider-handle-background:hovered")), t.at(tok!("slider-handle-background:pressed"))),
        handle_border: t.at(tok!("slider-handle-border")) }
}

pub(crate) const BODY: f32 = 14.0;
pub(crate) const SECOND: f32 = 12.0;
fn font(s: f32) -> FontId { FontId::proportional(s) }

/// Wrapped text with the given colour, returns its rect.
pub(crate) fn text_block(ui: &mut egui::Ui, text: &str, size: f32, col: Color32, indent: f32) -> egui::Response {
    let w = (ui.available_width() - indent).max(40.0);
    let g = ui.painter().layout(text.to_string(), font(size), col, w);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), g.size().y), Sense::hover());
    ui.painter().galley(r.min + Vec2::new(indent, 0.0), g, col);
    resp
}

/// Settings section: header (semibold), optional description (secondary), then the content.
pub(crate) fn section(ui: &mut egui::Ui, t: &Th, title: &str, desc: &str, first: bool, add: impl FnOnce(&mut egui::Ui)) {
    if !first { ui.add_space(crate::ui::theme::density::m().settings_section_gap); } // Standard 16
    if !title.is_empty() { ui.label(RichText::new(title).font(F::Title.semibold()).color(t.text1)); ui.add_space(4.0); }
    if !desc.is_empty() { text_block(ui, desc, SECOND, t.text2, 0.0); ui.add_space(4.0); }
    add(ui);
}

/// A check box row. `child` indents by 24. Returns true when toggled.
pub(crate) fn check(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, label: &str, on: &mut bool, enabled: bool, child: bool) -> bool {
    let k = tok(t);
    let indent = if child { 24.0 } else { 0.0 };
    let width = ui.available_width();
    // Disabled: checkbox-*-text disabled = white / black 40 %.
    let base = if is_dark(t) { Color32::WHITE } else { Color32::BLACK };
    let g = ui.painter().layout(label.to_string(), font(BODY), if enabled { t.text1 } else { alpha(base, 0.4) }, (width - indent - 32.0 - 4.0).max(40.0));
    let h = (g.size().y + 2.0).max(crate::ui::theme::density::m().settings_row_min);
    // Left margin -6 and indicator margin 8 -> the box sits 2 px from the section's left edge.
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, h), if enabled { Sense::click() } else { Sense::hover() });
    let b = Rect::from_min_size(pos2(r.left() + indent + 2.0, r.top() + (h - 16.0) / 2.0), Vec2::splat(16.0));
    let (p, c) = (ui.painter(), ctl(t));
    // Checked: fill + border checkbox-checked-background/-border (same values), tick checkbox-checked-icon;
    // unchecked: checkbox-unchecked-background (transparent / 7 % / 20 %) inside a checkbox-unchecked-border frame;
    // disabled: checked fill 20 %, tick 40 %, unchecked border 20 %.
    if *on { p.rect_filled(b, 2.0, if enabled { c.checked_bg.pick(&resp) } else { alpha(base, 0.2) }); }
    else { p.rect_filled(b, 2.0, if enabled { c.unchecked_bg.pick(&resp) } else { Color32::TRANSPARENT }); p.rect_stroke(b, 2.0, Stroke::new(1.0_f32, if enabled { c.unchecked_border } else { alpha(base, 0.2) }), egui::StrokeKind::Inside); }
    if *on { ic.paint(ui, "check-bold", Rect::from_center_size(b.center(), Vec2::splat(12.0)), if enabled { c.checked_icon } else { alpha(base, 0.4) }); }
    ui.painter().galley(pos2(r.left() + indent + 26.0, r.top() + 1.0), g, t.text1);
    if resp.has_focus() { ui.painter().rect_stroke(b.expand(2.0), 3.0, Stroke::new(2.0_f32, k.focus), egui::StrokeKind::Outside); }
    if enabled && resp.clicked() { *on = !*on; return true; }
    false
}

/// A small info button: a small info icon (`info_16`) whose hover shows `text` in a popover at most 320 wide.
pub(crate) fn more_info(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, text: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    ic.paint(ui, "info_16", Rect::from_center_size(r.center(), Vec2::splat(14.0)), if resp.hovered() { t.text1 } else { t.text2 });
    let text = text.to_string();
    resp.on_hover_ui(move |ui| { ui.set_max_width(320.0); ui.label(RichText::new(text).size(SECOND)); })
}

/// A checkbox whose label is followed by a more-info button (row spacing 4).
pub(crate) fn check_more(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, label: &str, on: &mut bool, info: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        // `check` wraps its label at width - 36 (box column 32 + 4): reserve that plus a pixel of rounding
        let w = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font(BODY), t.text1).size().x) + 37.0;
        let avail = ui.available_width() - 24.0;
        ui.allocate_ui(Vec2::new(w.min(avail), 24.0), |ui| { changed = check(ui, ic, t, label, on, true, false); });
        more_info(ui, ic, t, info);
    });
    changed
}

/// A checkbox row's description under it (checkBoxWithDescription: left margin 32).
pub(crate) fn check_desc(ui: &mut egui::Ui, t: &Th, text: &str) { ui.add_space(2.0); text_block(ui, text, SECOND, t.text2, 26.0); }

/// A radio button row; returns true when clicked.
pub(crate) fn radio(ui: &mut egui::Ui, t: &Th, label: &str, selected: bool, desc: &str) -> bool {
    let k = ctl(t);
    let width = ui.available_width();
    let g = ui.painter().layout(label.to_string(), font(BODY), t.text1, (width - 24.0 - 12.0).max(40.0));
    let dg = (!desc.is_empty()).then(|| ui.painter().layout(desc.to_string(), font(SECOND), t.text2, (width - 24.0).max(40.0)));
    let h = (g.size().y + 2.0).max(crate::ui::theme::density::m().settings_row_min) + dg.as_ref().map_or(0.0, |d| d.size().y); // Standard 24
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, h), Sense::click());
    let c = pos2(r.left() + 8.0, r.top() + crate::ui::theme::density::m().settings_row_min / 2.0); // Standard 12
    let p = ui.painter();
    if selected {
        p.circle_filled(c, 8.0, k.checked_bg.pick(&resp)); // radio-checked-background (= radio-checked-border)
        p.circle_filled(c, 3.0, k.checked_icon); // radio-checked-icon
    } else {
        p.circle_filled(c, 8.0, k.unchecked_bg.pick(&resp)); // radio-unchecked-background
        p.circle_stroke(c, 7.5, Stroke::new(1.0_f32, k.unchecked_border)); // radio-unchecked-border
    }
    p.galley(pos2(r.left() + 24.0, r.top() + 1.0), g.clone(), t.text1);
    if let Some(d) = dg { p.galley(pos2(r.left() + 24.0, r.top() + 1.0 + g.size().y), d, t.text2); }
    resp.clicked() && !selected
}

/// A combo box (the shared `menu::dropdown`). Returns the picked index.
pub(crate) fn combo(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, id: &str, width: f32, options: &[String], cur: usize) -> Option<usize> {
    let mut sel = cur;
    let r = crate::ui::menu::dropdown(ui, ic, t, &format!("settings-{id}"), &mut sel, options, width);
    r.changed().then_some(sel)
}

/// A slider (track 2 px / radius 2, handle 16) over `from..=to`; `steps` > 0 snaps to that many positions. Returns true on change.
pub(crate) fn slider(ui: &mut egui::Ui, t: &Th, v: &mut f32, from: f32, to: f32, steps: usize, width: f32) -> bool {
    let k = ctl(t);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width.min(ui.available_width()), 20.0), Sense::click_and_drag());
    let (x0, x1) = (r.left() + 8.0, r.right() - 8.0);
    let before = *v;
    if let Some(pos) = resp.interact_pointer_pos() { if resp.dragged() || resp.clicked() {
        let mut f = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        if steps > 1 { f = (f * (steps - 1) as f32).round() / (steps - 1) as f32; }
        *v = from + f * (to - from);
    } }
    let f = ((*v - from) / (to - from)).clamp(0.0, 1.0);
    let hx = x0 + f * (x1 - x0);
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(x0, r.center().y - 1.0), pos2(x1, r.center().y + 1.0)), 2.0, k.track_empty); // slider-track-empty-background
    p.rect_filled(Rect::from_min_max(pos2(x0, r.center().y - 1.0), pos2(hx, r.center().y + 1.0)), 2.0, k.track_indicator); // slider-track-indicator-background
    p.circle_filled(pos2(hx, r.center().y), 8.0, k.handle_bg.pick(&resp)); // slider-handle-background
    p.circle_stroke(pos2(hx, r.center().y), 7.5, Stroke::new(1.0_f32, k.handle_border)); // slider-handle-border
    (*v - before).abs() > f32::EPSILON
}

/// A pill button, secondary style: S = 28 high, XS = 24, outline, hover fill button-secondary.hovered.
pub(crate) fn pill(ui: &mut egui::Ui, t: &Th, label: &str, small: bool, enabled: bool) -> egui::Response {
    let k = tok(t);
    let h = if small { 28.0 } else { 24.0 };
    let w = ui.fonts(|f| f.layout_no_wrap(label.to_string(), semibold(BODY), t.text1).size().x) + 32.0;
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, h), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled { Color32::TRANSPARENT } else if resp.is_pointer_button_down_on() { k.btn_press } else if resp.hovered() { k.btn_hover } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(r, h / 2.0, fill);
    ui.painter().rect_stroke(r, h / 2.0, Stroke::new(1.0_f32, if enabled { k.outline_hover } else { k.outline }), egui::StrokeKind::Inside);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, semibold(BODY), if enabled { t.text1 } else { t.text2 });
    resp
}

/// A link: link colour, underlined while hovered.
pub(crate) fn link(ui: &mut egui::Ui, t: &Th, text: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), font(BODY), t.link);
    let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
    ui.painter().galley(r.min, g, t.link);
    if resp.hovered() { ui.painter().hline(r.x_range(), r.bottom() - 1.0, Stroke::new(1.0_f32, t.link)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    resp
}

/// A plain text field (32 high, radius 8, inputBackground / inputOutline, focus ring).
pub(crate) fn text_field(ui: &mut egui::Ui, t: &Th, text: &mut String, hint: &str, width: f32) -> egui::Response {
    let k = tok(t);
    let (r, _) = ui.allocate_exact_size(Vec2::new(width.min(ui.available_width()), 32.0), Sense::hover());
    ui.painter().rect_filled(r, 8.0, k.input_bg);
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(Vec2::new(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = c.add(egui::TextEdit::singleline(text).frame(false).margin(Vec2::ZERO).hint_text(RichText::new(hint).color(t.text2).size(BODY)).font(font(BODY)).desired_width(r.width() - 24.0));
    ui.painter().rect_stroke(r, 8.0, Stroke::new(if resp.has_focus() { 2.0_f32 } else { 1.0_f32 }, if resp.has_focus() { k.focus } else if resp.hovered() { k.outline_hover } else { k.outline }), egui::StrokeKind::Inside);
    resp
}
