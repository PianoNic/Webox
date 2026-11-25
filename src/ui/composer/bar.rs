//! The composer's bars: the toolbar row above the text (with Cancel / Update while editing) and the send pill (send | schedule).
use super::*;

/// Composer toolbar: glyph size and the gap between the 28 pt buttons (pitch 30 pt).
const TOOL_GLYPH: f32 = 16.0;
const TOOL_GAP: f32 = 2.0;

impl App {
    /// The toolbar row (part of `composer`, draw.rs). Returns the emoji and GIF button rects (popup anchors) and whether Update was clicked.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn composer_toolbar(&mut self, ui: &mut egui::Ui, t: &Th, crect: Rect, off: f32, st: &mut St, id: &String, ctx: &egui::Context, light: bool, edit: bool, now: f64) -> (Rect, Rect, bool) {
        let cid = egui::Id::new(CID);
        let mut update_hint = false;
        // Toolbar (action bar).
        let tb = Rect::from_min_size(pos2(crect.left() + 25.0, crect.top() + off + crate::ui::theme::density::m().composer_tools_y), Vec2::new(crect.width() - 50.0, 28.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(tb).layout(egui::Layout::left_to_right(egui::Align::Center)));
        // Pitch 30 pt between glyph centres = 28 button + 2, glyphs 16 pt.
        c.spacing_mut().item_spacing.x = TOOL_GAP;
        let md_tip = format!("{} (Alt+Shift+M)", if st.md { tr("Turn off Markdown") } else { tr("Turn on Markdown") });
        // Toolbar icons: attachment, screenshot, markdown, emoji, GIF, mention, and the AI sparkle, in the order and with the
        // buttons chosen in Settings > Customize the interface (the shortcuts work without a button). Send = send-filled, schedule =
        // recents-bold. The AI sparkle is drawn dimmed (#6F6F6F) as a disabled button (no hover, no click): the AI Assistant is out
        // of scope. Bitmoji (needs a Snapchat login) and video clips (Vidcast) are out of scope and not shown.
        use crate::ui::customize::{self as cz, COMPOSER as G};
        // A popup opened by its shortcut while its button is hidden opens at the toolbar's first slot.
        let first = Rect::from_min_size(tb.min, Vec2::splat(28.0));
        let mut emoji_anchor = first;
        let mut gif_anchor = first;
        st.legend_anchor = Some(first);
        for e in cz::visible(&self.settings, &G) {
            let (icon, tip): (&'static str, String) = match e.id {
                "attach" => ("attachment_16_em", format!("{} (Ctrl+O)", tr("Attach a file"))), "screenshot" => ("screenshot_16_em", tr("Screen Capture").into()), "markdown" => ("markdown_16_em", md_tip.clone()),
                "emoji" => ("emoticons_16_em", format!("{} (Ctrl+Shift+Space)", tr("Emoji and symbols"))), "gif" => ("gif_16", tr("Add a gif").into()),
                "mention" => ("mention_16_em", tr("@Mentions").into()), _ => ("sparkle-bold", tr("AI Assistant").into()),
            };
            if icon == "sparkle-bold" && id == super::super::new_message::DRAFT_ID { continue; } // no AI button in "New message"
            // The M button keeps the normal icon colour in both states (no accent).
            let tint = if icon == "sparkle-bold" { hex(0x6F6F6F) } else { hex(0xF3F3F3) };
            let tint = if light && icon != "sparkle-bold" { hex(0x121212) } else { tint };
            if icon == "sparkle-bold" { // disabled: drawn, not interactive
                let (r, _) = c.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                self.icons.paint(&mut c, icon, Rect::from_center_size(r.center(), Vec2::splat(TOOL_GLYPH)), tint);
                continue;
            }
            let bresp = self.icons.button(&mut c, icon, 28.0, TOOL_GLYPH, tint, t.hover).on_hover_text(crate::shortcuts::native_hint(&tip));
            if icon == "emoticons_16_em" { emoji_anchor = bresp.rect; }
            if icon == "gif_16" { gif_anchor = bresp.rect; if super::super::gif_picker::gif_picker_open() { c.painter().circle_filled(bresp.rect.center(), 14.0, t.ink_a(0.12)); } }
            if icon == "markdown_16_em" { st.legend_anchor = Some(bresp.rect); }
            if bresp.clicked() {
                match icon {
                    "emoticons_16_em" => self.emoji_picker_toggle(&ctx),
                    "gif_16" => self.gif_picker_toggle(&ctx),
                    "mention_16_em" => {
                        let d = self.drafts.entry(id.clone()).or_default();
                        let (_, p) = get_sel(&ctx, d.chars().count());
                        let ins = if p == 0 || d.chars().nth(p - 1).is_some_and(|c| c.is_whitespace()) { "@" } else { " @" };
                        let b = ct::byte_of(d, p);
                        d.insert_str(b, ins);
                        let np = p + ins.chars().count();
                        set_sel(&ctx, np, np);
                        ctx.memory_mut(|m| m.request_focus(cid));
                    }
                    "markdown_16_em" => st.legend = !st.legend,
                    "attachment_16_em" => self.attach_pick(&id, now),
                    "screenshot_16_em" => self.screen_capture_start(&id, true, now),
                    _ => self.toast(crate::trq!("%1 is not available in this version", tip), now),
                }
            }
        }
        if edit {
            // Cancel / Update at the right of the toolbar.
            let (ur, cr) = (Rect::from_center_size(pos2(tb.right() - 52.0, tb.center().y), Vec2::new(104.0, 28.0)), Rect::from_center_size(pos2(tb.right() - 104.0 - 8.0 - 46.0, tb.center().y), Vec2::new(92.0, 28.0)));
            let (ru, rc) = (ui.interact(ur, ui.id().with("edit-update"), Sense::click()), ui.interact(cr, ui.id().with("edit-cancel"), Sense::click()));
            ui.painter().rect_filled(ur, 14.0, if ru.hovered() { t.white_btn.gamma_multiply(0.85) } else { t.white_btn });
            ui.painter().text(ur.center(), egui::Align2::CENTER_CENTER, tr("Update"), F::Label.semibold(), t.white_btn_text);
            ui.painter().rect_stroke(cr, 14.0, Stroke::new(1.0_f32, t.sep), egui::StrokeKind::Inside);
            if rc.hovered() { ui.painter().rect_filled(cr, 14.0, t.hover); }
            ui.painter().text(cr.center(), egui::Align2::CENTER_CENTER, tr("Cancel"), F::Label.semibold(), t.text1);
            if ru.clicked() { update_hint = true; }
            if rc.clicked() { self.composer_escape(st, id); st.emoji_q = None; st.ment_q = None; }
        } else {
            // 16 px like the body text.
            c.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(tr("Shift+Enter for a new line")).color(t.text2).size(16.0)));
        }
        (emoji_anchor, gif_anchor, update_hint)
    }

    /// The send pill / send button (part of `composer`, draw.rs). Returns whether Send was clicked.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn composer_send_buttons(&mut self, ui: &mut egui::Ui, t: &Th, pr: Rect, light: bool, st: &mut St, id: &String, ctx: &egui::Context, edit: bool, now: f64) -> bool {
        let mut now_send = false;
        // "New message": one round send button where the schedule half sits, no schedule.
        if id == super::super::new_message::DRAFT_ID {
            let cr = Rect::from_min_max(pos2(pr.center().x, pr.top()), pr.max);
            let s = ui.interact(cr, ui.id().with("send"), Sense::click());
            crate::ui::theme::fx::send_button(cr);
            ui.painter().circle_stroke(cr.center(), pr.height() / 2.0 - 0.5, Stroke::new(1.0_f32, if light { hex(0xCCCCCC) } else { hex(0x3B3B3B) }));
            self.icons.paint(ui, "send-filled", Rect::from_center_size(cr.center(), Vec2::splat(16.0)), if s.hovered() { t.text1 } else { hex(0x6F6F6F) });
            if s.on_hover_text(tr("Send")).clicked() { now_send = true; }
        } else {
        // Send pill: send | schedule.
        ui.painter().rect_stroke(pr, 13.0, Stroke::new(1.0_f32, if light { hex(0xCCCCCC) } else { hex(0x3B3B3B) }), egui::StrokeKind::Inside);
        ui.painter().vline(pr.center().x, pr.y_range().shrink(6.0), Stroke::new(1.0_f32, if light { hex(0xCCCCCC) } else { hex(0x3B3B3B) }));
        let sr = Rect::from_min_max(pr.min, pos2(pr.center().x, pr.bottom()));
        let cr = Rect::from_min_max(pos2(pr.center().x, pr.top()), pr.max);
        // States: both halves are disabled (dim #6F6F6F) until there is something to send; "Send later" needs text and isn't
        // offered while editing. Enabled glyphs: label-primary.
        let has_text = self.drafts.get(id).is_some_and(|d| !d.trim().is_empty());
        let can_send = has_text || !st.chips.is_empty() || self.gif_staged(&id) || (!edit && self.quote_waiting(&id));
        let can_sched = has_text && !edit;
        let s = ui.interact(sr, ui.id().with("send"), if can_send { Sense::click() } else { Sense::hover() });
        let k = ui.interact(cr, ui.id().with("later"), if can_sched { Sense::click() } else { Sense::hover() });
        let dim = hex(0x6F6F6F);
        let on = |hov: bool| if hov { t.text1 } else { t.text1.gamma_multiply(0.85) };
        crate::ui::theme::fx::send_button(sr);
        if can_send && crate::ui::theme::decor::on(collabhost_plugin_api::deco::SEND_BUTTON) {
            // a soft gradient under the glyph (it stays readable) and a gradient outline round the pill
            crate::ui::theme::decor::fill(ui.painter(), sr.shrink(2.0), 11.0, &crate::ui::theme::decor::stripes(), 0.0, 0.32);
            crate::ui::theme::decor::outline(ui.painter(), pr, 13.0, 1.5, &crate::ui::theme::decor::stripes(), 0.0);
        }
        if can_send && s.hovered() { ui.painter().rect_filled(sr.shrink(2.0), egui::CornerRadius { nw: 11, sw: 11, ne: 0, se: 0 }, t.hover); }
        if can_sched && k.hovered() { ui.painter().rect_filled(cr.shrink(2.0), egui::CornerRadius { nw: 0, sw: 0, ne: 11, se: 11 }, t.hover); }
        self.icons.paint(ui, "send-filled", Rect::from_center_size(sr.center(), Vec2::splat(16.0)), if can_send { on(s.hovered()) } else { dim });
        self.icons.paint(ui, "recents-bold", Rect::from_center_size(cr.center(), Vec2::splat(16.0)), if can_sched { on(k.hovered()) } else { dim });
        if s.on_hover_text(tr("Send")).clicked() && can_send { now_send = true; }
        let k_clicked = k.on_hover_text(tr("Send later")).clicked() && can_sched;
        if let Some(due) = self.sched_menu(&ctx, t, cr, k_clicked) {
            // The composer's message is kept for `due` (ui/schedule_send.rs); emoticons and mentions as when sending.
            let mut text = self.drafts.get(id).cloned().unwrap_or_default();
            if self.emoticons_on() { text = ct::parse_emoticons(&text); }
            if !text.trim().is_empty() {
                let refs = ct::live_mentions(&text, &st.mentions).into_iter().map(|m| crate::richtext::MentionRef { id: m.id.clone(), name: m.name.clone(), all: m.all }).collect();
                self.sched_add(&id, text.trim().to_string(), refs, due, now);
                self.drafts.insert(id.clone(), String::new());
                st.mentions.clear(); st.emoji_q = None; st.ment_q = None;
                self.save_drafts();
            }
        }
        } // end of the two send variants
        now_send
    }
}
