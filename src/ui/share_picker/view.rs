//! The share dialog's layout. Window background theme-gradient-primary (border wx-searchInputBox-border, radius 8); title bar
//! 40 px; body 8 px below it: the source grid (left margin 24, at least 538 wide), a 12 px splitter, the options panel
//! (280 wide, theme-gradient-secondary, radius 12); bottom bar 80 px.
//! Source cell: 200 x 156 with 4 px spacing (196 x 152 drawn), 2 px border radius 12 (sharewindow-viewwindow-selected when
//! picked, -hovered on hover), preview 18 px in from the top / left / right on background-quaternary (radius 6, picture
//! fitted), the name below in 32 px (windows: 40 px, two lines). No picture: the application icon (24 px, text-secondary).
//! Options panel: title "Sharing options" 12 px in, then the entries 12 px below it, 8 px apart.
use super::{key, Act, Picker};
use crate::media::share::{Kind, Optimize, ShareSource};
use crate::*;

pub(super) const SIZE: Vec2 = Vec2::new(1080.0, 772.0);
pub(super) const MIN: Vec2 = Vec2::new(832.0, 584.0);
const TITLE_H: f32 = 40.0;
const BOTTOM_H: f32 = 80.0;
const CELL: Vec2 = Vec2::new(200.0, 156.0);
const CELL_GAP: f32 = 4.0;
const PANEL_W: f32 = 280.0;
const HANDLE_W: f32 = 12.0;

/// The "Application" section (windows; on Wayland the portal's window entry).
pub(super) fn is_app(s: &ShareSource) -> bool { s.kind == Kind::Window || (s.kind == Kind::Portal && s.handle == 2) }

/// German wording for the optimisation entries where the language pack has none.

impl App {
    /// The dialog's content in `r`. `own_window`: the title bar drags the window.
    pub(super) fn share_picker_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &Th, p: &mut Picker, r: Rect, own_window: bool) -> Act {
        let mut act = Act::None;
        let painter = ui.painter().clone();
        vquad(&painter, r, t.tk("theme-gradient-primary-0-background"), t.tk("theme-gradient-primary-1-background"));
        painter.rect_stroke(r, 8.0, Stroke::new(1.0f32, t.tk("wx-searchInputBox-border")), egui::StrokeKind::Inside);
        // title bar
        let bar = Rect::from_min_size(r.min, Vec2::new(r.width(), TITLE_H));
        let drag = ui.interact(bar, ui.id().with("share-title"), Sense::click_and_drag());
        if own_window && drag.drag_started() { crate::ui::chrome::note_os_grab(ctx); ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        painter.text(pos2(bar.left() + 16.0, bar.center().y), egui::Align2::LEFT_CENTER, crate::tr!("ShareDialogTitleBar", "Share content"), semibold(14.0), t.text1);
        let x = Rect::from_center_size(pos2(bar.right() - 24.0, bar.center().y), Vec2::splat(28.0));
        let xr = ui.interact(x, ui.id().with("share-close"), Sense::click());
        if xr.hovered() { painter.rect_filled(x, 14.0, t.tk("listItem-primary:hovered")); }
        let (c, d) = (x.center(), 5.0);
        painter.line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], Stroke::new(1.3f32, t.text1));
        painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], Stroke::new(1.3f32, t.text1));
        if xr.clicked() || ctx.input(|i| i.key_pressed(Key::Escape)) { act = Act::Close; }
        // body: grid | splitter | options
        let body = Rect::from_min_max(pos2(r.left(), bar.bottom() + 8.0), pos2(r.right() - 1.0, r.bottom() - BOTTOM_H));
        let grid_r = if p.options { Rect::from_min_max(body.min, pos2(body.right() - PANEL_W - HANDLE_W, body.bottom())) } else { body };
        if self.share_grid(ui, t, p, grid_r) { act = Act::Share; }
        if p.options { self.share_options(ui, t, p, Rect::from_min_max(pos2(body.right() - PANEL_W, body.top()), body.max)); }
        // bottom bar: "Share" centred (200 wide, XL), the options toggle at the right
        let bottom = Rect::from_min_max(pos2(r.left(), r.bottom() - BOTTOM_H), r.max);
        let share_r = Rect::from_center_size(bottom.center(), Vec2::new(200.0, 40.0));
        let enabled = p.sel.is_some();
        let sr = ui.interact(share_r, ui.id().with("share-go"), if enabled { Sense::click() } else { Sense::hover() });
        let (fill, fg) = if t.light { (hex(0x121212), Color32::WHITE) } else { (hex(0xF2F3F1), hex(0x121212)) };
        let fill = if !enabled { alpha(fill, 0.2) } else if sr.hovered() { alpha(fill, 0.85) } else { fill };
        painter.rect_filled(share_r, 20.0, fill);
        painter.text(share_r.center(), egui::Align2::CENTER_CENTER, crate::tr!("FileTabController", "Share"), semibold(16.0), if enabled { fg } else { alpha(fg, 0.6) });
        if sr.clicked() || (enabled && ctx.input(|i| i.key_pressed(Key::Enter))) { act = Act::Share; }
        let label = if p.options { crate::tr!("ShareDialogBottomView", "Hide sharing options") } else { crate::tr!("ShareDialogBottomView", "Show sharing options") };
        let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), semibold(14.0), t.text1).size().x);
        let tog = Rect::from_min_size(pos2(bottom.right() - 16.0 - tw - 52.0, bottom.center().y - 20.0), Vec2::new(tw + 52.0, 40.0));
        let tr = ui.interact(tog, ui.id().with("share-opts"), Sense::click());
        if tr.hovered() { painter.rect_filled(tog, 20.0, t.tk("listItem-primary:hovered")); }
        self.icons.paint(ui, "side-panel-bold", Rect::from_center_size(pos2(tog.left() + 26.0, tog.center().y), Vec2::splat(18.0)), t.text1);
        painter.text(pos2(tog.left() + 42.0, tog.center().y), egui::Align2::LEFT_CENTER, label, semibold(14.0), t.text1);
        if tr.clicked() { p.options = !p.options; }
        act
    }

    /// The two sections of source cells. True on a double click (share at once).
    fn share_grid(&mut self, ui: &mut egui::Ui, t: &Th, p: &mut Picker, r: Rect) -> bool {
        let inner = Rect::from_min_max(pos2(r.left() + 24.0, r.top()), r.max);
        let cols = ((inner.width() / CELL.x).floor() as usize).max(1);
        let sections: [(String, Vec<ShareSource>); 2] = [
            (crate::tr!("ScreenAndAppTab", "Screen").into(), p.sources.iter().filter(|s| !is_app(s)).cloned().collect()),
            (crate::tr!("ScreenAndAppTab", "Application").into(), p.sources.iter().filter(|s| is_app(s)).cloned().collect()),
        ];
        if p.sources.is_empty() {
            if let Some(m) = crate::media::share::unavailable() { child_note(ui, t, r, &m); return false; }
        }
        let height: f32 = sections.iter().filter(|s| !s.1.is_empty()).map(|(_, v)| 28.0 + 8.0 + v.len().div_ceil(cols) as f32 * CELL.y + 8.0).sum();
        let wheel = if ui.rect_contains_pointer(r) { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
        p.scroll = crate::ui::menu::wheel_offset(p.scroll, wheel, height - r.height());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        child.set_clip_rect(r);
        let mut y = inner.top() - p.scroll;
        let mut go = false;
        for (title, list) in sections.iter() {
            if list.is_empty() { continue; }
            child.painter().text(pos2(inner.left(), y + 14.0), egui::Align2::LEFT_CENTER, title, semibold(16.0), t.tk("label-primary-text"));
            y += 28.0 + 8.0;
            for (i, s) in list.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(inner.left() + (i % cols) as f32 * CELL.x, y + (i / cols) as f32 * CELL.y), CELL - Vec2::splat(CELL_GAP));
                if cell.bottom() < r.top() || cell.top() > r.bottom() { continue; }
                go |= self.share_cell(&mut child, t, p, s, cell);
            }
            y += list.len().div_ceil(cols) as f32 * CELL.y + 8.0;
        }
        go
    }

    /// One source cell. True on a double click.
    fn share_cell(&mut self, ui: &mut egui::Ui, t: &Th, p: &mut Picker, s: &ShareSource, cell: Rect) -> bool {
        let k = key(s);
        let resp = ui.interact(cell, ui.id().with(("share-cell", k)), Sense::click());
        if resp.clicked() { p.sel = Some(k); }
        let selected = p.sel == Some(k);
        let painter = ui.painter().clone();
        let border = if selected { Some(t.tk("sharewindow-viewwindow-selected")) } else if resp.hovered() { Some(t.tk("sharewindow-viewwindow-hovered")) } else { None };
        if let Some(c) = border { painter.rect_stroke(cell, 12.0, Stroke::new(2.0f32, c), egui::StrokeKind::Inside); }
        let app = is_app(s);
        let name_h = if app { 40.0 } else { 32.0 };
        let prev = Rect::from_min_max(pos2(cell.left() + 18.0, cell.top() + 18.0), pos2(cell.right() - 18.0, cell.bottom() - name_h));
        painter.rect_filled(prev, 6.0, t.tk("background-quaternary"));
        match p.tex.get(&k) {
            Some(tex) => {
                let sz = tex.size_vec2();
                let k2 = (prev.width() / sz.x).min(prev.height() / sz.y);
                let img = Rect::from_center_size(prev.center(), sz * k2);
                painter.image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None if p.no_pic.contains(&k) || s.kind == Kind::Portal => {
                let icon = if app { "application-bold" } else { "share-screen-bold" };
                self.icons.paint(ui, icon, Rect::from_center_size(prev.center(), Vec2::splat(24.0)), t.tk("text-secondary"));
            }
            None => { ui.put(Rect::from_center_size(prev.center(), Vec2::splat(32.0)), egui::Spinner::new().size(24.0).color(t.text2)); }
        }
        // the name: one line for screens, two for windows, elided
        let mut job = egui::text::LayoutJob::simple(s.name.clone(), FontId::proportional(13.0), t.tk("label-primary-text"), cell.width() - 8.0);
        job.halign = egui::Align::Center;
        job.wrap.max_rows = if app { 2 } else { 1 };
        job.wrap.break_anywhere = false;
        let g = ui.fonts(|f| f.layout_job(job));
        let ty = prev.bottom() + 4.0 + (name_h - 4.0 - if app { 4.0 } else { 6.0 } - g.size().y) / 2.0;
        painter.galley(pos2(cell.center().x, ty), g, t.text1);
        resp.double_clicked()
    }

    /// "Sharing options": optimise for text / motion, share the computer's sound.
    fn share_options(&mut self, ui: &mut egui::Ui, t: &Th, p: &mut Picker, r: Rect) {
        let painter = ui.painter().clone();
        // radius 12 on the left; the right edge runs into the window border
        let bg = Rect::from_min_max(r.min, pos2(r.right() + 12.0, r.bottom() + 12.0));
        let clip = painter.with_clip_rect(r);
        clip.rect_filled(bg, 12.0, t.tk("theme-gradient-secondary-0-background"));
        vquad(&clip, Rect::from_min_max(pos2(r.left() + 12.0, r.top()), r.max), t.tk("theme-gradient-secondary-0-background"), t.tk("theme-gradient-secondary-1-background"));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(Vec2::new(12.0, 12.0))));
        c.label(RichText::new(crate::trq!("Sharing options")).font(semibold(14.0)).color(t.text1));
        c.add_space(12.0);
        let opts = [crate::ui::widgets::tr("Optimize for text and images").to_string(), crate::ui::widgets::tr("Optimize for motion and video").to_string()];
        let cur = if p.optimize == Optimize::Motion { 1 } else { 0 };
        if let Some(i) = crate::ui::settings::kit::combo(&mut c, &mut self.icons, t, "share-optimize", r.width() - 24.0, &opts, cur) {
            p.optimize = if i == 1 { Optimize::Motion } else { Optimize::Text };
            if p.optimize == Optimize::Motion { p.audio = true; } // motion and video share the computer's sound with it
        }
        c.add_space(4.0);
        let hint = if p.optimize == Optimize::Motion { crate::trq!("Best for video and motion") } else { crate::trq!("Best for text and images") };
        c.label(RichText::new(hint).size(12.0).color(t.text2));
        c.add_space(8.0);
        crate::ui::settings::kit::check(&mut c, &mut self.icons, t, &crate::tr!("LastNewShareWidget", "Share computer audio"), &mut p.audio, true, false);
        if p.optimize == Optimize::Motion && p.audio {
            c.add_space(4.0);
            c.label(RichText::new(crate::trq!("You're optimizing for motion and video, so your computer audio is automatically shared.")).size(12.0).color(t.text2));
        }
    }
}

/// One muted line at the top of the (empty) source list.
fn child_note(ui: &mut egui::Ui, t: &Th, r: Rect, msg: &str) {
    ui.painter().text(pos2(r.left() + 24.0, r.top() + 14.0), egui::Align2::LEFT_CENTER, msg, FontId::proportional(14.0), t.text2);
}
