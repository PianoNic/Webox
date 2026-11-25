//! The reactors popover: hovering (or clicking) the count of a reaction chip lists who reacted that way.
//! Matches the Webex desktop layout: a balloon 350 pt wide above the count with a 12 pt arrow pointing at it, corner radius
//! 10, 8 pt above and below the rows, 48 pt per row (32 pt avatar, name, time, presence line, the emoji), at most 5 rows
//! visible (the rest scroll). Background popover-primary-background, border list-header-outline-secondary (ink @ 20 %).
//! Leaving the count closes it after 100 ms unless the pointer is in the popover; leaving the popover closes it after 500 ms.
use super::*;
use cloud::Reactor;
use egui::Pos2;

const W: f32 = 350.0;
const ROW: f32 = 48.0;
const MARGIN: f32 = 8.0;
const ARROW: f32 = 12.0;
const MAX_ROWS: usize = 5;
/// The target is the count label grown by 2 pt on every side.
const TARGET_PAD: f32 = 2.0;
const CLOSE_FROM_COUNT: f64 = 0.1;
const CLOSE_FROM_POPOVER: f64 = 0.5;

/// One row of the popover.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ReactorRow { pub id: String, pub email: String, pub name: String, pub me: bool, pub published: String }

/// The rows for a reaction's users, in the service's order (each person once). My own row is labelled `you` ("Sie");
/// others are looked up in `known` by id or e-mail (the reactor's id is a uuid, the people lists' ids are e-mail-style, so the
/// e-mail the session resolved for it is what matches), then the name the session resolved; a person not found is `unknown`.
pub(super) fn reactor_rows(users: &[Reactor], me: &Person, known: &[Person], you: &str, unknown: &str) -> Vec<ReactorRow> {
    let same = |id: &str, p: &Person| !id.is_empty() && ((!p.id.is_empty() && p.id.eq_ignore_ascii_case(id)) || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(id)));
    let is = |u: &Reactor, p: &Person| same(&u.id, p) || same(&u.email, p);
    let mut out: Vec<ReactorRow> = vec![];
    for u in users {
        if u.id.is_empty() || out.iter().any(|r| r.id == u.id) { continue }
        let mine = is(u, me);
        let p = if mine { Some(me) } else { known.iter().find(|p| is(u, p)) };
        let name = if mine { you.to_string() } else {
            p.map(|p| p.name.clone()).filter(|n| !n.is_empty()).or_else(|| Some(u.name.clone()).filter(|n| !n.trim().is_empty())).unwrap_or_else(|| unknown.to_string())
        };
        let email = p.map(|p| p.email.clone()).filter(|e| !e.is_empty()).unwrap_or_else(|| u.email.clone());
        out.push(ReactorRow { id: u.id.clone(), email, name, me: mine, published: u.published.clone() });
    }
    out
}

/// When a person reacted, stamped like a message header: "HH:MM" today and yesterday, else "13.11.2026, 16:48".
/// `local`: turn the UTC time into local time first (real data; the offline sample keeps its own clock).
pub(super) fn reacted_time(published: &str, local: bool, today: &str, yesterday: &str) -> String {
    let (Some(d), Some(t)) = (published.get(..10), published.get(11..16)) else { return String::new() };
    let (d, t) = if local { to_local(d, t).unwrap_or((d.to_string(), t.to_string())) } else { (d.to_string(), t.to_string()) };
    header_time(&d, &t, today, yesterday)
}

/// The balloon for `rows` rows at the count label `count`: (body, arrow tip, above). Centred on the count, kept 8 pt inside
/// the window; above the count, below it only when there is no room above.
pub(super) fn popover_geom(count: Rect, rows: usize, screen: Rect) -> (Rect, Pos2, bool) {
    let t = count.expand(TARGET_PAD);
    let h = MARGIN * 2.0 + ROW * rows.clamp(1, MAX_ROWS) as f32;
    let x = (t.center().x - W / 2.0).clamp(screen.left() + 8.0, (screen.right() - W - 8.0).max(screen.left() + 8.0));
    let above = t.top() - ARROW - h >= screen.top();
    if above { (Rect::from_min_size(pos2(x, t.top() - ARROW - h), Vec2::new(W, h)), pos2(t.center().x, t.top()), true) }
    else { (Rect::from_min_size(pos2(x, t.bottom() + ARROW), Vec2::new(W, h)), pos2(t.center().x, t.bottom()), false) }
}

/// The close deadline after this frame: none while the pointer is on the count or the popover; otherwise the one already
/// running, or a new one (100 ms after leaving the count, 500 ms after leaving the popover).
pub(super) fn close_deadline(cur: Option<f64>, over_count: bool, over_pop: bool, came_from_pop: bool, now: f64) -> Option<f64> {
    if over_count || over_pop { return None }
    Some(cur.unwrap_or(now + if came_from_pop { CLOSE_FROM_POPOVER } else { CLOSE_FROM_COUNT }))
}

/// The open popover: which chip, where its count is, the close deadline, whether the pointer was last in the popover.
#[derive(Clone, Debug)]
struct Pop { msg: String, name: String, count: Rect, close_at: Option<f64>, in_pop: bool, pinned: bool, pass: u64 }

fn key() -> Id { Id::new("reactors-popover") }

impl App {
    /// Called by the chip row for the count part of a chip: opens the popover on hover or click and keeps its anchor current
    /// while the list scrolls. `WEBOX_SHOT_REACTORS=<reaction name>` opens it for a screenshot.
    pub(super) fn reactors_hover(&mut self, ctx: &egui::Context, m: &Message, r: &cloud::Reaction, count: Rect, resp: &egui::Response) {
        if r.users.is_empty() { return }
        let shot = self.shot.is_some() && std::env::var("WEBOX_SHOT_REACTORS").is_ok_and(|v| v == r.name);
        let mut cur: Option<Pop> = ctx.data(|d| d.get_temp(key()));
        let this = cur.as_ref().is_some_and(|p| p.msg == m.id && p.name == r.name);
        let pass = ctx.cumulative_pass_nr();
        if resp.hovered() || resp.clicked() || (shot && !this) {
            cur = Some(Pop { msg: m.id.clone(), name: r.name.clone(), count, close_at: None, in_pop: false, pinned: shot, pass });
        } else if this {
            if let Some(p) = cur.as_mut() { p.count = count; p.pass = pass; }
        } else { return }
        ctx.data_mut(|d| d.insert_temp(key(), cur.unwrap()));
    }

    /// Draws the open popover (once per frame, after the list) and closes it on time or with Esc.
    pub(super) fn reactors_popover(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, msgs: &[Message]) {
        let ctx = ui.ctx().clone();
        let Some(mut pop) = ctx.data(|d| d.get_temp::<Pop>(key())) else { return };
        // The chip scrolled out of view (or its space was left): no anchor this frame, so the popover goes.
        if pop.pass != ctx.cumulative_pass_nr() { ctx.data_mut(|d| d.remove::<Pop>(key())); return }
        let Some(r) = msgs.iter().find(|m| m.id == pop.msg).and_then(|m| m.reactions.iter().find(|r| r.name == pop.name)).cloned() else {
            ctx.data_mut(|d| d.remove::<Pop>(key())); return
        };
        let known = self.reactor_people();
        let unknown = crate::i18n::tr("ConversationRenderer", "Unknown").into_owned();
        let rows = reactor_rows(&r.users, &self.me, &known, &crate::i18n::tr("QObject", "You"), &unknown);
        if rows.is_empty() || ctx.input(|i| i.key_pressed(Key::Escape)) { ctx.data_mut(|d| d.remove::<Pop>(key())); return }
        // Someone not known here (not in the space any more, never wrote in the loaded history): their names are read from the
        // message's reaction activities once; the list is folded again with them.
        if rows.iter().any(|r| !r.me && r.name == unknown) {
            if let (Some(c), Some(space)) = (&self.client, self.selected.clone()) {
                let ask = format!("{space}/{}", pop.msg);
                let asked = ctx.data_mut(|d| !d.get_temp_mut_or_default::<std::collections::HashSet<String>>(Id::new("reactor-names-asked")).insert(ask));
                if !asked { c.req(cloud::Req::ReactorNames(space, pop.msg.clone())); }
            }
        }
        let (body, tip, above) = popover_geom(pop.count, rows.len(), ctx.screen_rect());
        // Pointer: on the count, in the popover (arrow included), or neither.
        let hover = ctx.pointer_hover_pos();
        let over_count = hover.is_some_and(|p| pop.count.expand(TARGET_PAD).contains(p));
        let outer = body.union(Rect::from_center_size(tip, Vec2::splat(1.0)));
        let over_pop = hover.is_some_and(|p| outer.contains(p));
        let now = ctx.input(|i| i.time);
        if !pop.pinned { pop.close_at = close_deadline(pop.close_at, over_count, over_pop, pop.in_pop, now); }
        if over_pop { pop.in_pop = true } else if over_count { pop.in_pop = false }
        if pop.close_at.is_some_and(|at| now >= at) { ctx.data_mut(|d| d.remove::<Pop>(key())); return }
        if let Some(at) = pop.close_at { ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0))); }
        ctx.data_mut(|d| d.insert_temp(key(), pop.clone()));

        let dark = !t.light;
        let ink = if dark { Color32::WHITE } else { Color32::BLACK };
        let (bg, border) = (t.tk("popover-primary-background"), wa(ink, 0.2));
        let glyph = chip_emoji(&r.name);
        let (today, yesterday) = day_ref(self.client.is_some());
        let local = self.client.is_some();
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        egui::Area::new(Id::new("reactors-popover-area")).fixed_pos(outer.min).order(egui::Order::Foreground).interactable(true).show(&ctx, |ui| {
            ui.set_clip_rect(outer.expand(24.0));
            ui.allocate_exact_size(outer.size(), Sense::hover());
            let pt = ui.painter().clone();
            pt.add(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if dark { 140 } else { 40 }) }.as_shape(body, 10.0));
            pt.rect_filled(body, 10.0, bg);
            pt.rect_stroke(body, 10.0, Stroke::new(1.0_f32, border), StrokeKind::Inside);
            // The arrow: 45° sides, its tip at the count's (grown) edge.
            let base = if above { body.bottom() } else { body.top() };
            let inward = if above { -1.0 } else { 1.0 };
            pt.add(Shape::convex_polygon(vec![pos2(tip.x - ARROW, base + inward), pos2(tip.x + ARROW, base + inward), tip], bg, Stroke::NONE));
            pt.line_segment([pos2(tip.x - ARROW, base), tip], Stroke::new(1.0_f32, border));
            pt.line_segment([pos2(tip.x + ARROW, base), tip], Stroke::new(1.0_f32, border));
            let list = Rect::from_min_max(pos2(body.left(), body.top() + MARGIN), pos2(body.right(), body.bottom() - MARGIN));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
            child.set_clip_rect(list);
            crate::ui::widgets::vscroll().id_salt(("reactors-scroll", &pop.msg, &pop.name)).max_height(list.height()).auto_shrink([false; 2]).show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0; // rows on a 48 pt pitch
                for row in &rows { self.reactor_row(ui, t, c, row, glyph.as_deref(), &today, &yesterday, local, now_ms); }
            });
        });
    }

    /// One 48 pt row: avatar 32 pt at 16 pt, the name (16 pt) and the time (secondary, right, 16 pt in) on the first line,
    /// the presence line (secondary) and the emoji (16 pt, right) on the second.
    #[allow(clippy::too_many_arguments)]
    fn reactor_row(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, row: &ReactorRow, glyph: Option<&str>, today: &str, yesterday: &str, local: bool, now_ms: i64) {
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
        let person = if row.me { self.me.clone() } else { self.people.iter().find(|p| p.id == row.id || (!row.email.is_empty() && p.email == row.email)).cloned()
            .unwrap_or(Person { id: row.id.clone(), name: row.name.clone(), email: row.email.clone(), status: String::new() }) };
        let ph = self.photo_for(ui.ctx(), &person.id, &person.email);
        let ar = Rect::from_min_size(pos2(r.left() + 16.0, r.center().y - 16.0), Vec2::splat(32.0));
        let mut av = ui.new_child(egui::UiBuilder::new().max_rect(ar));
        let avatar_name = if row.me { self.me.name.clone() } else { row.name.clone() };
        avatar_photo(&mut av, &mut self.icons, t, &avatar_name, 32.0, false, None, Some(person.status.as_str()).filter(|s| !s.is_empty()), ph);
        let (x, right) = (ar.right() + 12.0, r.right() - 16.0);
        let status = self.presence_line(&person, now_ms);
        let (l1, l2) = if status.is_empty() && glyph.is_none() { (r.center().y, r.center().y) } else { (r.top() + 15.0, r.top() + 33.0) };
        let time = reacted_time(&row.published, local, today, yesterday);
        let tg = ui.painter().layout_no_wrap(time, FontId::proportional(SECOND), t.text2);
        ui.painter().galley(pos2(right - tg.size().x, l1 - tg.size().y / 2.0), tg.clone(), t.text2);
        let name_w = (right - tg.size().x - 8.0 - x).max(20.0);
        let ng = ui.painter().layout(row.name.clone(), FontId::proportional(BODY), c.body, f32::INFINITY);
        let ng = if ng.size().x > name_w { ui.painter().layout_job(elided(&row.name, BODY, c.body, name_w)) } else { ng };
        ui.painter().galley(pos2(x, l1 - ng.size().y / 2.0), ng, c.body);
        let er = Rect::from_center_size(pos2(right - 8.0, l2), Vec2::splat(16.0));
        if let Some(g) = glyph { if !self.icons.emoji(ui, g, er) { ui.painter().text(er.center(), Align2::CENTER_CENTER, g, FontId::proportional(14.0), c.body); } }
        if !status.is_empty() {
            let sg = ui.painter().layout_job(elided(&status, SECOND, t.text2, (er.left() - 8.0 - x).max(20.0)));
            ui.painter().galley(pos2(x, l2 - sg.size().y / 2.0), sg, t.text2);
        }
    }

    /// Everyone a reactor can be: the open space's full participant list (asked for once, read-only), the people list, me.
    pub(super) fn reactor_people(&mut self) -> Vec<Person> {
        let mut out = self.selected.clone().and_then(|id| self.spaces.iter().find(|s| s.id == id).cloned()).map(|s| self.space_people(&s)).unwrap_or_default();
        out.extend(self.people.iter().cloned());
        out
    }
}

/// A single line cut with "…" at `w`.
fn elided(s: &str, size: f32, color: Color32, w: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(s.to_string(), FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping { max_width: w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, name: &str, email: &str) -> Person { Person { id: id.into(), name: name.into(), email: email.into(), status: String::new() } }
    fn u(id: &str, at: &str) -> Reactor { Reactor { id: id.into(), published: at.into(), ..Default::default() } }

    /// The real case: reactors are uuids, the people lists use e-mail-style ids. The e-mail / name the session resolved for
    /// the uuid is what finds them (this was "Unknown" for everyone).
    #[test]
    fn uuid_reactors_are_found_by_their_resolved_email_or_name() {
        let me = p("alex@example.com", "Alex Muster", "alex@example.com");
        let known = [p("erika@example.com", "Erika Beispiel", "erika@example.com")];
        let r = |id: &str, name: &str, email: &str| Reactor { id: id.into(), published: String::new(), name: name.into(), email: email.into() };
        let users = [r("u-erika", "Erika B.", "erika@example.com"), r("u-me", "", "alex@example.com"), r("u-gone", "Lea Weg", "lea@example.com"), r("u-none", "", "")];
        let rows = reactor_rows(&users, &me, &known, "You", "Unknown");
        let got: Vec<(&str, &str, bool)> = rows.iter().map(|r| (r.name.as_str(), r.email.as_str(), r.me)).collect();
        assert_eq!(got, [("Erika Beispiel", "erika@example.com", false), ("You", "alex@example.com", true), ("Lea Weg", "lea@example.com", false), ("Unknown", "", false)]);
    }

    #[test]
    fn rows_keep_the_service_order_and_call_me_you() {
        let me = p("me-uuid", "Alex Muster", "alex@example.com");
        let known = [p("u1", "Erika Beispiel", "erika@example.com"), p("max@example.com", "Max Probe", "max@example.com")];
        let users = [u("u1", "a"), u("me-uuid", "b"), u("max@example.com", "c"), u("u1", "d"), u("gone", "e")];
        let rows = reactor_rows(&users, &me, &known, "You", "Unknown");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Erika Beispiel", "You", "Max Probe", "Unknown"]); // each person once, unknown kept
        assert!(rows[1].me && !rows[0].me);
        assert_eq!(rows[0].published, "a");
        // My e-mail as the id (offline / e-mail style participant ids) is me too.
        assert!(reactor_rows(&[u("ALEX@example.com", "")], &me, &[], "Sie", "?")[0].me);
    }

    #[test]
    fn time_is_stamped_like_a_message_header() {
        assert_eq!(reacted_time("2026-11-20T09:03:00.000Z", false, "2026-11-20", "2026-11-19"), "09:03");
        assert_eq!(reacted_time("2026-11-19T23:59:00.000Z", false, "2026-11-20", "2026-11-19"), "23:59");
        assert_eq!(reacted_time("2026-11-13T16:48:00.000Z", false, "2026-11-20", "2026-11-19"), "13.11.2026, 16:48");
        assert_eq!(reacted_time("", false, "2026-11-20", "2026-11-19"), "");
    }

    #[test]
    fn balloon_sits_above_the_count_and_shows_five_rows_at_most() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let count = Rect::from_min_size(pos2(400.0, 500.0), Vec2::new(20.0, 22.0));
        let (body, tip, above) = popover_geom(count, 2, screen);
        assert!(above);
        assert_eq!((body.width(), body.height()), (350.0, 8.0 * 2.0 + 48.0 * 2.0));
        assert_eq!(tip, pos2(410.0, 498.0));
        assert_eq!(body.bottom(), 498.0 - 12.0);
        assert_eq!(body.center().x, 410.0);
        assert_eq!(popover_geom(count, 9, screen).0.height(), 8.0 * 2.0 + 48.0 * 5.0);
        // Near the left edge it stays 8 pt in; with no room above it opens below.
        assert_eq!(popover_geom(Rect::from_min_size(pos2(10.0, 500.0), Vec2::splat(20.0)), 1, screen).0.left(), 8.0);
        let (b, t, above) = popover_geom(Rect::from_min_size(pos2(400.0, 20.0), Vec2::splat(20.0)), 1, screen);
        assert!(!above && b.top() == 42.0 + 12.0 && t.y == 42.0);
    }

    #[test]
    fn closes_100_ms_after_the_count_and_500_ms_after_the_popover() {
        assert_eq!(close_deadline(None, true, false, false, 1.0), None);
        assert_eq!(close_deadline(Some(1.05), false, true, false, 1.0), None);
        assert_eq!(close_deadline(None, false, false, false, 1.0), Some(1.1));
        assert_eq!(close_deadline(None, false, false, true, 1.0), Some(1.5));
        assert_eq!(close_deadline(Some(1.1), false, false, false, 1.08), Some(1.1)); // a running deadline isn't pushed back
    }
}
