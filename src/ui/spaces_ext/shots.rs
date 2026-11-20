//! Screenshot-mode steps for the space features (only with --screenshot).
use super::*;

impl App {
    /// Screenshot-mode steps (only with `--screenshot`): WEBOX_SHOT_SPACES_DO="unread:s3,mute:s2" runs those menu actions at
    /// frame 35; WEBOX_SHOT_SPACES_DLG=notify:<space>|teampick:<space>|teamremove:<space>|archive:<space>|teamleave:<team>|
    /// teammembers:<team>|archiveteam:<team> opens that dialog at frame 40; WEBOX_SHOT_SPACES_PEOPLE=<n> opens the Personen
    /// tab's context menu of its n-th person at frame 45.
    pub(crate) fn spaces_shot(&mut self, ctx: &egui::Context, now: f64) {
        if self.shot.is_none() { return; }
        if self.frames == 35 {
            if let Ok(v) = std::env::var("WEBOX_SHOT_SPACES_DO") {
                for pair in v.split(',') {
                    let Some((a, sid)) = pair.split_once(':') else { continue };
                    if let Some(s) = self.spaces.iter().find(|s| s.id == sid).cloned() { self.space_action(ctx, a, &s, now); }
                }
            }
        }
        if self.frames == 40 {
            if let Some((k, id)) = std::env::var("WEBOX_SHOT_SPACES_DLG").ok().and_then(|v| v.split_once(':').map(|(a, b)| (a.to_string(), b.to_string()))) {
                let team = self.spaces.iter().find(|s| s.id == id).map(|s| s.team.clone()).unwrap_or_default();
                open_dialog(match k.as_str() {
                    "notify" => Dlg::Notify(id.clone(), self_notify_index(self.notify_of(&id))),
                    "teampick" => Dlg::TeamPick(id, self.teams.first().map(|t| t.id.clone())),
                    "teamremove" => Dlg::TeamRemove(id, team),
                    "archive" => Dlg::ArchiveSpace(id),
                    "teamleave" => Dlg::TeamLeave(id),
                    "archiveteam" => Dlg::ArchiveTeam(id),
                    _ => Dlg::TeamMembers(id, String::new()),
                });
            }
        }
        // WEBOX_SHOT_SPACES_ROW=<space> opens the row menu of that space (beside the list, as the "···" would).
        if self.frames == 41 {
            if let Ok(id) = std::env::var("WEBOX_SHOT_SPACES_ROW") {
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("row-menu-target"), (id, Rect::from_center_size(pos2(540.0, 251.0), Vec2::splat(24.0)))));
                menu::menu_open(ctx, "row");
            }
        }
        if self.frames == 45 {
            if let Some(n) = std::env::var("WEBOX_SHOT_SPACES_PEOPLE").ok().and_then(|v| v.parse::<usize>().ok()) {
                if let Some(s) = self.selected.clone().and_then(|id| self.spaces.iter().find(|s| s.id == id).cloned()) {
                    let list = self.space_people(&s);
                    if let Some(p) = list.get(n) {
                        let at = Rect::from_min_size(pos2(760.0, 240.0 + 51.0 * n as f32), Vec2::splat(1.0));
                        ctx.data_mut(|d| d.insert_temp(egui::Id::new("people-ctx-target"), (p.id.clone(), p.name.clone(), p.email.clone(), p.status.clone(), at)));
                        menu::menu_open(ctx, "people-ctx");
                    }
                }
            }
        }
    }
}
