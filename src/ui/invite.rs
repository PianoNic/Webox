//! "Invite to Webex" (header + > third entry), shown in the content pane. Layout (pt, from the content card's top / centre):
//! margins 60, a 26 pt semibold title, a 14 pt subtitle, a 480 wide search field, the message-send-320-cobalt-lime
//! illustration (163 x 132), L pills 8 apart.
//! Texts (language pack QObject): "Invite to {{APP_NAME}}", "Start a conversation with someone you're inviting to collaborate.",
//! "Enter email address", "Send invite", "Close". "Send invite" is disabled (grey) while the field holds no e-mail address
//! and white once one is typed (an address check). "Close" goes back to "Recommended messages"; the list selection is
//! cleared while the pane is open.
//! Send = the user's own click: `Req::Direct { invite: true }` (the e-mail gets a uuid with shouldCreateUsers, then the
//! one-to-one space), see cloud/header_plus.rs.
use crate::*;
use crate::cloud::Req;

#[derive(Clone, Default)]
struct Inv { open: bool, email: String, focus: bool, prev: Option<String> }

fn key() -> egui::Id { egui::Id::new("invite-state") }
fn get(ctx: &egui::Context) -> Inv { ctx.data(|d| d.get_temp::<Inv>(key())).unwrap_or_default() }
fn put(ctx: &egui::Context, s: Inv) { ctx.data_mut(|d| d.insert_temp(key(), s)); }
pub(crate) fn is_open(ctx: &egui::Context) -> bool { get(ctx).open }
pub(crate) fn close_state(ctx: &egui::Context) { put(ctx, Inv::default()); }
/// --sample screenshots only: pre-fill the field.
pub(crate) fn shot_text(ctx: &egui::Context, text: &str) { let mut s = get(ctx); s.email = text.into(); put(ctx, s); }

/// A plausible e-mail address (name@example.com): enables "Einladung senden".
pub(crate) fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    let Some((l, d)) = s.split_once('@') else { return false };
    !l.is_empty() && !s.contains(char::is_whitespace) && !d.contains('@') && d.split('.').count() >= 2 && d.split('.').all(|p| !p.is_empty())
}

// Geometry (pt): vertical centres from the content card's top, widths centred on the card.
const TITLE_Y: f32 = 78.0;
const SUB_Y: f32 = 106.5;
const SUB_PITCH: f32 = 19.2;
const FIELD_W: f32 = 480.0;
const FIELD_TOP: f32 = 146.4;
const FIELD_H: f32 = 32.0;
const ART_TOP: f32 = 272.4;
const ART: (f32, f32) = (163.0, 132.0);
/// Centre of the square the art is drawn into: the raster puts the 132x106 drawing in the upper part of its square, so the
/// square's centre sits 16 pt below the drawing's centre (342.8 pt).
const ART_CY: f32 = 358.6;
const BTN_TOP: f32 = 556.8;
const BTN_H: f32 = 32.0;
const BTN_GAP: f32 = 8.0;

impl App {
    pub(crate) fn invite_open(&mut self, ctx: &egui::Context) {
        super::new_message::set_state(ctx, Default::default());
        self.new_space = None;
        self.set_tab(Tab::Messaging);
        let prev = self.selected.take();
        put(ctx, Inv { open: true, focus: true, prev, ..Default::default() });
    }

    pub(crate) fn invite_screen(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut s = get(&ctx);
        let full = ui.max_rect();
        vgrad(ui.painter(), Rect::from_min_max(full.min, pos2(full.max.x + 10.0, full.max.y)), 10.0, t.conv, t.conv_b); // square top-right at the window edge, as the conversation card
        let cx = full.center().x;
        let y = |v: f32| full.top() + v;
        let p = ui.painter().clone();
        p.text(pos2(cx, y(TITLE_Y)), egui::Align2::CENTER_CENTER, super::header_plus::label("Invite to {{APP_NAME}}"), semibold(26.0), t.text1);
        let sub = crate::trq!("Start a conversation with someone you\u{2019}re inviting to collaborate.").into_owned();
        // Secondary text: 13.6 pt with this font gives a 368 pt first line, so the label wraps at 440 (narrower than the 480 pt
        // field) before "Zusammenarbeit".
        let g = ui.fonts(|f| f.layout(sub, FontId::proportional(13.6), t.text2, 440.0));
        for (i, row) in g.rows.iter().enumerate() {
            let line: String = row.glyphs.iter().map(|gl| gl.chr).collect();
            p.text(pos2(cx, y(SUB_Y + SUB_PITCH * i as f32)), egui::Align2::CENTER_CENTER, line.trim(), FontId::proportional(13.6), t.text2);
        }

        // Field with the search icon.
        let field = Rect::from_min_size(pos2(cx - FIELD_W / 2.0, y(FIELD_TOP)), Vec2::new(FIELD_W, FIELD_H));
        let tid = egui::Id::new("invite-field");
        p.rect_filled(field, 8.0, t.tk("modal-primary-background")); // the dialog's own colour (#000000 dark)
        p.rect_stroke(field, 8.0, Stroke::new(1.0_f32, t.ink_a(0.9)), egui::StrokeKind::Inside); // ink 90 % (focused field)
        self.icons.paint(ui, "search-regular", Rect::from_center_size(pos2(field.left() + 15.6, field.center().y), Vec2::splat(16.0)), t.text1);
        let clear_w = if s.email.is_empty() { 0.0 } else { 30.0 };
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(field.left() + 33.6, field.top()), pos2(field.right() - 10.0 - clear_w, field.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let te = c.add(egui::TextEdit::singleline(&mut s.email).id(tid).hint_text(RichText::new(crate::trq!("Enter email address")).color(t.text2).size(15.0))
            .frame(false).margin(Vec2::ZERO).font(FontId::proportional(15.0)).desired_width(f32::INFINITY));
        if std::mem::take(&mut s.focus) { te.request_focus(); }
        if !s.email.is_empty() {
            let xr = Rect::from_center_size(pos2(field.right() - 20.0, field.center().y), Vec2::splat(22.0));
            let xresp = ui.interact(xr, egui::Id::new("invite-clear"), Sense::click());
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(12.0)), t.text1);
            if xresp.clicked() { s.email.clear(); te.request_focus(); }
        }

        // The svg is 132x106 and rasterised into a square texture (icons.rs): draw a 163 square so the drawing keeps its aspect
        // (163 x 131).
        let _ = (ART_TOP, ART.1);
        self.icons.art(ui, "message-send-320-cobalt-lime", Rect::from_center_size(pos2(cx, y(ART_CY)), Vec2::splat(ART.0)));

        // Pills: "Send invite" (primary L) + "Close" (secondary L), centred as a pair.
        let send_label = crate::trq!("Send invite").into_owned();
        let close_label = crate::tr!("InviteToWebexDialog", "Close").into_owned();
        let w_of = |s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_string(), semibold(16.0), t.text1).size().x) + 28.0; // widths 152.8 / 92.8 pt
        let (w1, w2) = (w_of(&send_label), w_of(&close_label));
        let left = cx - (w1 + BTN_GAP + w2) / 2.0;
        let b1 = Rect::from_min_size(pos2(left, y(BTN_TOP)), Vec2::new(w1, BTN_H));
        let b2 = Rect::from_min_size(pos2(b1.right() + BTN_GAP, y(BTN_TOP)), Vec2::new(w2, BTN_H));
        let ok = looks_like_email(&s.email);
        let r1 = ui.interact(b1, egui::Id::new("invite-send"), if ok { Sense::click() } else { Sense::hover() });
        // button-primary-background / -text (disabled = 20 % / 40 %).
        let fill1 = if !ok { t.tk("button-primary-background:disabled") } else if r1.is_pointer_button_down_on() { t.tk("button-primary-background:pressed") } else if r1.hovered() { t.tk("button-primary-background:hovered") } else { t.white_btn };
        p.rect_filled(b1, BTN_H / 2.0, fill1);
        p.text(b1.center(), egui::Align2::CENTER_CENTER, &send_label, semibold(16.0), if ok { t.white_btn_text } else { t.tk("button-primary-text:disabled") });
        let r2 = ui.interact(b2, egui::Id::new("invite-close"), Sense::click());
        if r2.hovered() { p.rect_filled(b2, BTN_H / 2.0, t.hover); }
        p.rect_stroke(b2, BTN_H / 2.0, Stroke::new(1.0_f32, t.tk("button-secondary-border")), egui::StrokeKind::Inside);
        p.text(b2.center(), egui::Align2::CENTER_CENTER, &close_label, semibold(16.0), t.text1);
        if r1.hovered() || r2.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }

        let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if ok && (r1.clicked() || enter) {
            let email = s.email.trim().to_string();
            match &self.client {
                Some(c) => { c.req(Req::Direct { who: email.clone(), text: String::new(), invite: true }); self.expect_direct(&ctx, &email); }
                None => self.toast(format!("{} ({})", crate::trq!("Send invite"), tr("sample data")), now), // --sample: nothing leaves the process
            }
            close_state(&ctx);
            return;
        }
        if r2.clicked() {
            close_state(&ctx);
            // "Close" lands on "Recommended messages": its equivalent here is that filter.
            if let Some(id) = s.prev { if self.spaces.iter().any(|x| x.id == id) { self.open_space(id); } }
            self.filter = Filter::Notifications;
            return;
        }
        put(&ctx, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_needs_an_address() {
        assert!(looks_like_email("zzqxw@example.invalid"));
        assert!(looks_like_email(" a.b@c.example "));
        for bad in ["", "abc", "a@b", "a@b.", "@b.example", "a b@c.example", "a@b@c.example"] { assert!(!looks_like_email(bad), "{bad}"); }
    }
}
