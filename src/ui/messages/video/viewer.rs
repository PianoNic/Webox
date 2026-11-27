//! The full-screen viewer of the open video. The window goes full screen (and back to windowed on leaving, if it was), the
//! picture is contain-fit on black with the same control bar along its bottom and a close button top right; a click on the
//! picture plays / pauses, a double click or Esc leaves.
use super::*;
use super::card::pull_frame;

impl App {
    /// Draws the viewer when the open video is in viewer mode; called once per frame by the message list (after the rows).
    pub(in super::super) fn video_viewer(&mut self, ctx: &egui::Context, t: &Th) {
        let now = ctx.input(|i| i.time);
        let Some((entering, leaving)) = ACTIVE.with(|s| {
            let mut s = s.borrow_mut();
            let x = s.as_mut()?;
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            // entering: remember whether the window was windowed; leaving: restore it
            let entering = x.viewer && !x.restore_windowed && !fullscreen;
            let leaving = !x.viewer && x.restore_windowed;
            if entering { x.restore_windowed = true; }
            if leaving { x.restore_windowed = false; }
            Some((entering, leaving))
        }) else { return };
        if entering { ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true)); ctx.memory_mut(|m| m.stop_text_input()); }
        if leaving { ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false)); }
        if !ACTIVE.with(|s| s.borrow().as_ref().is_some_and(|x| x.viewer)) { return; }
        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("video-viewer")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::BLACK);
            let st = ACTIVE.with(|s| { let mut s = s.borrow_mut(); let x = s.as_mut()?; x.drawn = ctx.cumulative_pass_nr(); x.drawn_at = now; pull_frame(ctx, x); Some((x.tex.as_ref().map(|t| t.id()), x.size, x.player.playing(), x.player.ended(), x.player.info().error.is_some())) });
            let Some((tid, size, playing, ended, failed)) = st else { return };
            let c = mc(t);
            if failed {
                let er = Rect::from_center_size(r.center(), Vec2::new(420.0, 240.0));
                let _ = self.video_play_error(ui, t, &c, er, false);
            } else if let Some(tid) = tid {
                round_image(ui.painter(), card::contain(r, size[0], size[1]), 0.0, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tid, Color32::WHITE);
            } else { self.video_spinner(ui, &c, r.center(), None, now); }
            if (!playing || ended) && tid.is_some() {
                let br = Rect::from_center_size(r.center(), Vec2::splat(PLAY_XL * 1.5));
                ui.painter().circle_filled(br.center(), br.width() / 2.0, t.at(tok!("button-primary-background")));
                self.icons.paint(ui, "play-filled", Rect::from_center_size(br.center() + Vec2::new(2.0, 0.0), Vec2::splat(30.0)), t.at(tok!("button-primary-text")));
            }
            let bar = self.video_bar(ui, t, r, true, now);
            let close = Rect::from_min_size(pos2(r.right() - 52.0, r.top() + 14.0), Vec2::splat(36.0));
            let cs = ui.interact(close, Id::new("video-viewer-close"), Sense::click());
            if cs.hovered() { ui.painter().circle_filled(close.center(), 18.0, Color32::from_white_alpha(28)); }
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(close.center(), Vec2::splat(18.0)), Color32::WHITE);
            let leave = cs.on_hover_text(format!("{} (Esc)", crate::tr!("MeetingViewButtons", "Exit full screen"))).clicked() || resp.double_clicked();
            if !leave && resp.clicked() && !resp.interact_pointer_pos().is_some_and(|q| bar.contains(q)) {
                ACTIVE.with(|s| if let Some(x) = s.borrow().as_ref() { if x.player.ended() { x.player.seek(0.0); } x.player.toggle(); });
            }
            if leave { ACTIVE.with(|s| if let Some(x) = s.borrow_mut().as_mut() { x.viewer = false; }); ctx.request_repaint(); }
        });
        self.video_keys(ctx);
    }
}
