//! "Create a space": the content pane that replaces the chat pane when the + next to the list title
//! (or Ctrl+Shift+N) is used. Title, name field, people picker with chips, the campfire
//! illustration (campfire-gather-192-cobalt-lime), the public checkbox and Create / Close.
use crate::*;

/// Extra state of the screen, kept in egui's memory so the App struct stays untouched.
/// `team`: the space is created inside this team ("Create a space for this team"); set from the Teams page or when the list
/// shows a team. There is no team picker: the team is shown, not chosen.
#[derive(Clone, Default)]
struct Draft { public: bool, people: Vec<Person>, query: String, team: Option<String>, init: bool }

/// People to add once the new space shows up in the list (the create request only carries the title).
#[derive(Clone, Default)]
struct Pending { title: String, known: Vec<String>, emails: Vec<String> }

// Layout (pt, from the card origin): a top-down flow - title, subtitle, name field (500 x 28), people field, the
// illustration (200), then one bottom row centred in the card: [checkbox column (explanation indented 22)] [spacer]
// [Create >= 75 x 32] [20] [Close >= 75 x 32].
/// inputBackground (#121212 / #FFFFFF) with a 1 px border at 60 % ink.
fn outline_field(ui: &mut egui::Ui, t: &Th, rect: Rect) {
    ui.painter().rect_filled(rect, 14.0, t.tk("inputBackground"));
    ui.painter().rect_stroke(rect, 14.0, Stroke::new(1.0_f32, t.ink_a(0.6)), egui::StrokeKind::Inside);
}
/// The bottom row's left edge (the row is 577 pt wide and centred), and its top (y of the checkbox / buttons from the card top).
const ROW_W: f32 = 577.0;
const CHECK_Y: f32 = 526.0;
const BUTTON_Y: f32 = 547.0;

impl App {
    pub(crate) fn create_space_screen(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let full = ui.max_rect();
        vgrad(ui.painter(), Rect::from_min_max(full.min, pos2(full.max.x + 10.0, full.max.y)), 10.0, t.conv, t.conv_b); // square top-right at the window edge, as the conversation card
        let id = egui::Id::new("create-space-draft");
        let mut d: Draft = ui.data_mut(|m| m.get_temp(id)).unwrap_or_default();
        if !d.init { d.init = true; if d.team.is_none() { d.team = self.team_sel.clone(); } } // opened while a team is shown
        let mut name = self.new_space.clone().unwrap_or_default();
        let cx = full.center().x;
        let fw = (full.width() - 80.0).clamp(240.0, 500.0);

        ui.painter().text(pos2(cx, full.top() + 72.0), egui::Align2::CENTER_CENTER, tr("Create a space"), semibold(24.0), t.text1);
        // In a team the subtitle names the team, with an x to create the space outside it.
        let team_now = d.team.clone().and_then(|tid| self.teams.iter().find(|x| x.id == tid).cloned());
        if team_now.is_none() { ui.painter().text(pos2(cx, full.top() + 101.0), egui::Align2::CENTER_CENTER, tr("Start a group conversation with others"), FontId::proportional(14.0), t.text2); }
        if let Some(tm) = team_now {
            let label = format!("{}: {}", tr("Create a space for this team"), tm.name);
            let g = ui.painter().text(pos2(cx, full.top() + 101.0), egui::Align2::CENTER_CENTER, label, FontId::proportional(14.0), team_tones(t, tm.color, &tm.id).1);
            let xr = Rect::from_center_size(pos2(g.right() + 12.0, g.center().y), Vec2::splat(16.0));
            let xresp = ui.interact(xr, egui::Id::new("create-team-x"), Sense::click());
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(10.0)), if xresp.hovered() { t.text1 } else { t.text2 });
            if xresp.clicked() { d.team = None; }
        }

        // Field 1: the name (required).
        let r1 = Rect::from_center_size(pos2(cx, full.top() + 135.0), Vec2::new(fw, 28.0));
        outline_field(ui, t, r1);
        let mut c1 = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r1.min + Vec2::new(30.0, 0.0), r1.max - Vec2::new(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let name_resp = c1.add(egui::TextEdit::singleline(&mut name).hint_text(RichText::new(tr("Name the space (required)")).color(t.text2).size(14.0))
            .frame(false).margin(Vec2::ZERO).font(FontId::proportional(14.0)).desired_width(r1.width() - 44.0));
        if !name_resp.has_focus() && !ui.memory(|m| m.has_focus(egui::Id::new("create-people"))) && name.is_empty() && self.new_space.as_deref() == Some("") { name_resp.request_focus(); }

        // Field 2: add people by name, number or e-mail address.
        let r2 = Rect::from_center_size(pos2(cx, full.top() + 170.0), Vec2::new(fw, 28.0));
        outline_field(ui, t, r2);
        self.icons.paint(ui, "search-regular", Rect::from_center_size(r2.left_center() + Vec2::new(14.0, 0.0), Vec2::splat(14.0)), t.text2);
        let mut c2 = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r2.min + Vec2::new(30.0, 0.0), r2.max - Vec2::new(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        c2.add(egui::TextEdit::singleline(&mut d.query).id(egui::Id::new("create-people")).hint_text(RichText::new(tr("Add by name, number, or email")).color(t.text2).size(14.0))
            .frame(false).margin(Vec2::ZERO).font(FontId::proportional(14.0)).desired_width(r2.width() - 48.0));

        // Matches from the contacts (self.people) under the field, then the chosen people as chips.
        let q = d.query.trim().to_lowercase();
        let mut y = r2.bottom() + 8.0;
        if !q.is_empty() {
            let hits: Vec<Person> = self.people.iter().filter(|p| !d.people.iter().any(|x| x.id == p.id)
                && (p.name.to_lowercase().contains(&q) || p.email.to_lowercase().contains(&q))).take(4).cloned().collect();
            for p in hits {
                let rr = Rect::from_min_size(pos2(r2.left(), y), Vec2::new(fw, 30.0));
                let resp = ui.interact(rr, egui::Id::new(("create-hit", &p.id)), Sense::click());
                if resp.hovered() { ui.painter().rect_filled(rr, 15.0, t.hover); }
                let av = Rect::from_center_size(rr.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(24.0));
                let mut ac = ui.new_child(egui::UiBuilder::new().max_rect(av));
                let ph = self.photo_for(ui.ctx(), &p.id, &p.email);
                avatar_photo(&mut ac, &mut self.icons, t, &p.name, 24.0, false, None, None, ph);
                ui.painter().text(pos2(rr.left() + 38.0, rr.center().y), egui::Align2::LEFT_CENTER, format!("{}   {}", p.name, p.email), FontId::proportional(13.5), t.text1);
                if resp.clicked() { d.people.push(p); d.query.clear(); }
                y += 32.0;
            }
        }
        if !d.people.is_empty() {
            let mut x = r2.left();
            let mut remove = None;
            for (i, p) in d.people.iter().enumerate() {
                let w = ui.fonts(|f| f.layout_no_wrap(p.name.clone(), FontId::proportional(13.5), t.text1).size().x) + 44.0;
                if x + w > r2.right() { x = r2.left(); y += 32.0; }
                let rr = Rect::from_min_size(pos2(x, y), Vec2::new(w, 26.0));
                ui.painter().rect_filled(rr, 13.0, t.selected);
                ui.painter().text(pos2(rr.left() + 12.0, rr.center().y), egui::Align2::LEFT_CENTER, &p.name, FontId::proportional(13.5), t.text1);
                let xr = Rect::from_center_size(pos2(rr.right() - 14.0, rr.center().y), Vec2::splat(18.0));
                let xresp = ui.interact(xr, egui::Id::new(("create-chip-x", &p.id)), Sense::click());
                self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(11.0)), if xresp.hovered() { t.text1 } else { t.text2 });
                if xresp.clicked() { remove = Some(i); }
                x += w + 6.0;
            }
            if let Some(i) = remove { d.people.remove(i); }
        }

        // The campfire illustration, centred in the lower middle (192 px asset drawn at 200).
        let ill = Rect::from_center_size(pos2(cx, full.top() + 378.0), Vec2::splat(200.0));
        if ill.bottom() < full.bottom() - 100.0 { self.icons.art(ui, "campfire-gather-192-cobalt-lime", ill); }

        // Bottom row (top-down flow, centred group, see ROW_W): public checkbox with its explanation, then the two buttons.
        let row_x = (cx - ROW_W / 2.0).max(full.left() + 12.0);
        let bl = pos2(row_x, full.top() + CHECK_Y);
        let cb = Rect::from_min_size(bl, Vec2::splat(16.0));
        // Signed in, the box is disabled: making a new space public is not implemented, so no guessed write goes to the real
        // account. With sample data it works locally. In a team the public option does not apply either.
        let can_public = self.client.is_none() && d.team.is_none();
        let cresp = ui.interact(Rect::from_min_size(bl, Vec2::new(280.0, 24.0)), egui::Id::new("create-public"), if can_public { Sense::click() } else { Sense::hover() });
        if cresp.clicked() { d.public = !d.public; }
        if !can_public { d.public = false; }
        let dim = |c: Color32| if can_public { c } else { c.gamma_multiply(0.45) };
        if d.public {
            ui.painter().rect_filled(cb, 4.0, t.white_btn);
            self.icons.paint(ui, "check-bold", Rect::from_center_size(cb.center(), Vec2::splat(12.0)), t.white_btn_text);
        } else {
            ui.painter().rect_stroke(cb, 4.0, Stroke::new(1.0_f32, dim(t.tk("checkbox-unchecked-border"))), egui::StrokeKind::Inside);
        }
        ui.painter().text(pos2(row_x + 24.0, cb.center().y), egui::Align2::LEFT_CENTER, tr("Make this space public"), FontId::proportional(14.0), dim(t.text1));
        // Explanation: indented 22, three lines (~236 wide).
        let mut job = egui::text::LayoutJob::simple(tr("Anyone in your organization can find and join a public space.").to_string(), FontId::proportional(12.5), t.text2, 236.0);
        job.wrap.max_width = 236.0;
        let g = ui.fonts(|f| f.layout_job(job));
        ui.painter().galley(pos2(row_x + 22.0, full.top() + 552.0), g, t.text2);

        // Bottom-right: Create (primary once a name is entered) and Close (outlined), 32 px pills.
        let ready = !name.trim().is_empty();
        let by = full.top() + BUTTON_Y;
        let make = Rect::from_min_size(pos2(row_x + 375.0, by), Vec2::new(86.0, 32.0));
        let close = Rect::from_min_size(pos2(row_x + 481.0, by), Vec2::new(96.0, 32.0));
        let mresp = ui.interact(make, egui::Id::new("create-make"), if ready { Sense::click() } else { Sense::hover() });
        // Disabled: button-primary-background / -text disabled = 20 % / 40 %.
        ui.painter().rect_filled(make, 16.0, if !ready { t.tk("button-primary-background:disabled") } else if mresp.hovered() { t.tk("button-primary-background:hovered") } else { t.white_btn });
        ui.painter().text(make.center(), egui::Align2::CENTER_CENTER, tr("Create"), semibold(14.0), if ready { t.white_btn_text } else { t.tk("button-primary-text:disabled") });
        let cresp2 = ui.interact(close, egui::Id::new("create-close"), Sense::click());
        if cresp2.hovered() { ui.painter().rect_filled(close, 16.0, t.hover); }
        ui.painter().rect_stroke(close, 16.0, Stroke::new(1.0_f32, t.tk("button-secondary-border")), egui::StrokeKind::Inside);
        ui.painter().text(close.center(), egui::Align2::CENTER_CENTER, tr("Close"), semibold(14.0), t.text1);

        let enter = name_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        if ready && (mresp.clicked() || enter) {
            let title = name.trim().to_string();
            // One create request carrying the chosen people, or a team space inside the team; with sample data a local space
            // (nothing leaves the process).
            let known: Vec<String> = self.spaces.iter().map(|s| s.id.clone()).collect();
            match self.create_space_with(&title, &d.people, d.team.as_deref(), d.public) {
                None => ui.data_mut(|m| m.insert_temp(egui::Id::new("create-pending"), Pending { title, known, emails: vec![] })),
                Some(sid) => { self.open_space(sid); self.toast(tr("Space created (sample data)"), now); }
            }
            self.new_space = None;
            ui.data_mut(|m| m.remove::<Draft>(id));
            return;
        }
        if cresp2.clicked() || esc {
            self.new_space = None;
            ui.data_mut(|m| m.remove::<Draft>(id));
            return;
        }
        self.new_space = Some(name);
        ui.data_mut(|m| m.insert_temp(id, d));
    }

    /// "Create a space for this team": opens this pane with the team preset.
    pub(crate) fn create_space_in_team(&mut self, ctx: &egui::Context, team: &str) {
        ctx.data_mut(|m| m.insert_temp(egui::Id::new("create-space-draft"), Draft { team: Some(team.to_string()), init: true, ..Default::default() }));
        self.tab = Tab::Messaging;
        self.team_sel = Some(team.to_string());
        self.new_space = Some(String::new());
    }

    /// After a create request: once the new space is in the list, add the chosen people and open it.
    pub(crate) fn finish_create_space(&mut self, ctx: &egui::Context) {
        let key = egui::Id::new("create-pending");
        let Some(p) = ctx.data(|m| m.get_temp::<Pending>(key)) else { return };
        if let Some(s) = self.spaces.iter().find(|s| s.title == p.title && !p.known.contains(&s.id)).cloned() {
            if let Some(c) = &self.client { for e in &p.emails { c.req(Req::AddPerson(s.id.clone(), e.clone())); } }
            self.open_space(s.id);
            ctx.data_mut(|m| m.remove::<Pending>(key));
        }
    }
}
