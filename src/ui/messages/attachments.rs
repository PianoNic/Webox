//! The attachments of a message row: pictures, video cards, the file cards' flow and link cards, measured and drawn.
use super::*;

/// The attachment behind each "📎" line (same order; by name when the counts differ). Video files leave the file cards and become
/// video cards. Returns (file card names, their attachments, video attachments).
pub(super) fn split_files(m: &Message, files: Vec<String>) -> (Vec<String>, Vec<Option<Attachment>>, Vec<Attachment>) {
    let n = files.len();
    let (mut names, mut atts, mut videos) = (vec![], vec![], vec![]);
    for (fi, f) in files.into_iter().enumerate() {
        let att = if m.files.len() == n { m.files.get(fi) } else { m.files.iter().find(|a| a.name == f) };
        match att.filter(|a| !a.key.is_empty() && crate::media::video::format::is_video(&a.mime, &f)) {
            Some(a) => videos.push(a.clone()),
            None => { names.push(f); atts.push(att.cloned()); }
        }
    }
    (names, atts, videos)
}

impl App {
    /// Height of the attachments with the gaps between them (`first`: nothing above yet).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn attachments_height(&mut self, ui: &egui::Ui, c: &Mc, m: &Message, videos: &[Attachment], files: usize, wrap: f32, first: &mut bool) -> f32 {
        let mut h = 0.0;
        let mut gap = |h: &mut f32| { if !*first { *h += attach_gap(); } *first = false; };
        for a in &m.images {
            gap(&mut h);
            h += if is_video(a) { video::card_size(a, wrap, self.video_known_size(ui.ctx(), a)).y } else { image_box(a, wrap, self.photos.image_size(ui.ctx(), &a.key)).y };
        }
        for a in videos { gap(&mut h); h += video::card_size(a, wrap, self.video_known_size(ui.ctx(), a)).y; }
        if files > 0 { gap(&mut h); h += file_flow(files, wrap).2; } // the files' flow (gallery view)
        for p in &m.previews { gap(&mut h); h += Self::card_parts(ui, c, p, wrap.min(CARD_MAX_W)).3.h; }
        h
    }

    /// Draws the attachments from `at` down and moves `y` below them. Returns how many blocks there were (pictures, videos, the
    /// files' flow as one block, link cards).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_attachments(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, m: &Message, ct: &Content, at: egui::Pos2, wrap: f32, y: &mut f32) -> usize {
        let tx = at.x;
        let n_attach = m.images.len() + ct.videos.len() + usize::from(!ct.files.is_empty()) + m.previews.len();
        let (mut idx, mut block) = (0usize, 0usize);
        let mut next = |y: &mut f32, h: f32| { block += 1; *y += h + if block < n_attach { attach_gap() } else { ATTACH_BOTTOM }; };
        for a in &m.images {
            let h = if is_video(a) { self.video_block(ui, t, c, pos2(tx, *y), a, wrap, (&m.id, idx), true) } else { self.image_block(ui, c, pos2(tx, *y), a, wrap, (&m.id, idx)) };
            idx += 1;
            next(y, h);
        }
        for a in &ct.videos { let h = self.video_block(ui, t, c, pos2(tx, *y), a, wrap, (&m.id, idx), false); idx += 1; next(y, h); }
        if !ct.files.is_empty() {
            // The file cards side by side in a flow (spacing 8), wrapping when the message is narrow.
            let (pos, cw, fh) = file_flow(ct.files.len(), wrap);
            for (fi, f) in ct.files.iter().enumerate() {
                let att = ct.file_atts.get(fi).and_then(|a| a.as_ref());
                let (dx, dy) = pos[fi];
                self.file_chip(ui, t, c, Rect::from_min_size(pos2(tx + dx, *y + dy), Vec2::new(cw, FILE_CARD_H)), f, att.map_or(0, |a| a.size), (&m.id, idx), att.map(|a| a.key.as_str()));
                idx += 1;
            }
            next(y, fh);
        }
        for pv in &m.previews { let h = self.link_card(ui, c, pos2(tx, *y), wrap, pv, (&m.id, idx)); idx += 1; next(y, h); }
        n_attach
    }
}
