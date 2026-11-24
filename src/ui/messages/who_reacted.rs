//! Who reacted, at a glance: small round avatars of the people behind a reaction, drawn inside the chip after its count, then
//! "+K" for the rest. Off unless a plugin turns it on (`reactions.show_avatars`, the WhoReacted plugin). Names and e-mails come
//! with the reactors (`Reactor.name` / `.email`, resolved by the cloud layer) or from the caches the reactors popover uses;
//! hovering an avatar shows the person's name. No network requests of its own.
use super::*;
use crate::extras::plugins::{ReactionAvatars, REACTION_AVATARS_MAX};
use cloud::Reactor;

/// How much of an avatar the next one covers.
const OVERLAP: f32 = 0.3;
/// Space between the count and the first avatar, and between the last avatar and "+K".
const GAP: f32 = 4.0;
const MORE_FONT: f32 = 11.0;

/// Which reactors a chip shows (indices into `users`, at most `REACTION_AVATARS_MAX`) and how many more there are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ChipAvatars { idx: [u16; REACTION_AVATARS_MAX], pub n: usize, pub more: usize }
impl ChipAvatars {
    pub fn shown(&self) -> &[u16] { &self.idx[..self.n] }
}

/// Whether `id` (a participant uuid or e-mail) is `p`.
fn same(id: &str, p: &Person) -> bool { !id.is_empty() && ((!p.id.is_empty() && p.id.eq_ignore_ascii_case(id)) || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(id))) }
/// Whether reactor `u` is `p`, by uuid or e-mail (the same match as the popover).
fn is(u: &Reactor, p: &Person) -> bool { same(&u.id, p) || same(&u.email, p) }

/// The first `a.max` people of a reaction (each once, in the service's order; mine left out with `hide_self`) and the "+K":
/// everyone else the count stands for (the service may list fewer people than the count). No allocation.
pub(super) fn chip_avatars(users: &[Reactor], count: u32, mine: bool, me: &Person, a: ReactionAvatars) -> ChipAvatars {
    let mut out = ChipAvatars { idx: [0; REACTION_AVATARS_MAX], n: 0, more: 0 };
    let (mut listed, mut me_listed) = (0usize, false);
    for (i, u) in users.iter().enumerate() {
        if u.id.is_empty() || users[..i].iter().any(|v| v.id == u.id) { continue }
        if is(u, me) { me_listed = true; if a.hide_self { continue } }
        listed += 1;
        if out.n < a.max.min(REACTION_AVATARS_MAX) { out.idx[out.n] = i as u16; out.n += 1; }
    }
    let hidden_me = a.hide_self && (mine || me_listed);
    out.more = (count as usize).saturating_sub(hidden_me as usize).max(listed) - out.n;
    out
}

/// Width of the avatars (and "+K") for `n` avatars of `size`, without the gap before them; 0 for none.
pub(super) fn avatars_width(n: usize, size: f32, more_w: f32) -> f32 {
    let w = if n == 0 { 0.0 } else { size + (n - 1) as f32 * size * (1.0 - OVERLAP) };
    if more_w > 0.0 { w + if n == 0 { 0.0 } else { GAP } + more_w } else { w }
}

/// A known person for reactor `u`: me, the people list, the open space's participants (borrowed, nothing cloned or fetched).
fn known<'a>(me: &'a Person, people: &'a [Person], parts: &'a [Person], u: &Reactor) -> Option<&'a Person> {
    if is(u, me) { return Some(me) }
    people.iter().chain(parts).find(|p| is(u, p))
}

/// What a chip needs to lay out its avatars: which ones, the "+K" text, and the width they take (incl. the gap before them).
pub(super) struct ChipLayout { pub av: ChipAvatars, pub size: f32, pub more: Option<Arc<Galley>>, pub width: f32 }

impl App {
    /// The avatars of reaction `r` if a plugin turned them on (`cfg`), measured for the chip; None draws today's chip.
    pub(super) fn chip_layout(&self, ui: &egui::Ui, c: &Mc, r: &cloud::Reaction, cfg: Option<ReactionAvatars>) -> Option<ChipLayout> {
        let a = cfg?;
        let av = chip_avatars(&r.users, r.count, r.mine.is_some(), &self.me, a);
        if av.n == 0 && av.more == 0 { return None }
        let more = (av.more > 0).then(|| ui.painter().layout_no_wrap(format!("+{}", av.more), FontId::proportional(MORE_FONT), c.head));
        let width = GAP + avatars_width(av.n, a.size, more.as_ref().map_or(0.0, |g| g.size().x));
        Some(ChipLayout { av, size: a.size, more, width })
    }

    /// Draws the chip's avatars from `x` (vertically centred on `cy`); hovering one names the person like the popover does.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn chip_avatars_draw(&mut self, ui: &mut egui::Ui, t: &Th, c: &Mc, m: &Message, r: &cloud::Reaction, ri: usize, l: &ChipLayout, x: f32, cy: f32) {
        let ctx = ui.ctx().clone();
        let sq = cur_seq(&ctx);
        let step = l.size * (1.0 - OVERLAP);
        let parts: &[Person] = self.selected.as_deref().and_then(|id| self.spaces.iter().find(|s| s.id == id)).map_or(&[], |s| &s.participants);
        let mut ax = x + GAP;
        let mut hovered = None;
        for (k, &i) in l.av.shown().iter().enumerate() {
            let u = &r.users[i as usize];
            let rect = Rect::from_min_size(pos2(ax, cy - l.size / 2.0), Vec2::splat(l.size));
            let p = known(&self.me, &self.people, parts, u);
            let email = p.map(|p| p.email.as_str()).filter(|e| !e.is_empty()).unwrap_or(&u.email);
            let name = p.map(|p| p.name.as_str()).filter(|n| !n.is_empty()).unwrap_or(&u.name);
            let ph = [u.id.as_str(), email].into_iter().filter(|k| !k.is_empty()).find_map(|k| self.photos.texture(&ctx, k));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            avatar_photo(&mut child, &mut self.icons, t, name, l.size, false, None, None, ph);
            // A ring in the card colour keeps overlapping avatars apart.
            if k > 0 { ui.painter().circle_stroke(rect.center(), l.size / 2.0 + 0.5, Stroke::new(1.5_f32, t.card)); }
            let resp = ui.interact(rect, Id::new(("react-av", &m.id, &r.name, ri, k, sq)), Sense::hover());
            if resp.hovered() { hovered = Some((resp, i as usize)); }
            ax += step;
        }
        if let Some((resp, i)) = hovered {
            // Only under the pointer: the popover's own lookup, so a name it resolves shows here too.
            let known = self.reactor_people();
            let row = reactors::reactor_rows(std::slice::from_ref(&r.users[i]), &self.me, &known, &crate::i18n::tr("QObject", "You"), &crate::i18n::tr("ConversationRenderer", "Unknown"));
            if let Some(row) = row.first() { resp.on_hover_text(row.name.clone()); }
        }
        if let Some(g) = &l.more {
            let gx = if l.av.n == 0 { x + GAP } else { ax - step + l.size + GAP };
            ui.painter().galley(pos2(gx, cy - g.size().y / 2.0), g.clone(), c.head);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(id: &str) -> Reactor { Reactor { id: id.into(), ..Default::default() } }
    fn me() -> Person { Person { id: "me-uuid".into(), name: "Alex".into(), email: "alex@example.com".into(), status: String::new() } }
    const A: ReactionAvatars = ReactionAvatars { max: 3, size: 14.0, hide_self: false };

    #[test]
    fn shows_up_to_max_then_plus_the_rest() {
        let users = [u("a"), u("b"), u("a"), u("c"), u("d"), u("")];
        let av = chip_avatars(&users, 4, false, &me(), A);
        assert_eq!((av.shown(), av.more), (&[0u16, 1, 3][..], 1)); // each person once, empty ids skipped
        assert_eq!(chip_avatars(&users, 4, false, &me(), ReactionAvatars { max: 10, ..A }).more, 0);
        assert_eq!(chip_avatars(&users, 4, false, &me(), ReactionAvatars { max: 1, ..A }).shown(), &[0u16]);
        // The count is bigger than the list the service sent: the rest is "+K" too.
        assert_eq!(chip_avatars(&[u("a")], 6, false, &me(), A).more, 5);
        // No users at all: just "+count".
        let none = chip_avatars(&[], 2, false, &me(), A);
        assert_eq!((none.n, none.more), (0, 2));
    }

    #[test]
    fn hide_self_leaves_my_reaction_out() {
        let users = [u("ALEX@example.com"), u("b")];
        let on = ReactionAvatars { hide_self: true, ..A };
        assert_eq!(chip_avatars(&users, 2, true, &me(), A).shown(), &[0u16, 1]);
        let av = chip_avatars(&users, 2, true, &me(), on);
        assert_eq!((av.shown(), av.more), (&[1u16][..], 0));
        // Only I reacted: nothing to show.
        let av = chip_avatars(&[u("me-uuid")], 1, true, &me(), on);
        assert_eq!((av.n, av.more), (0, 0));
        // Matched by the e-mail the cloud layer resolved, too.
        let by_mail = Reactor { id: "other-uuid".into(), email: "alex@example.com".into(), ..Default::default() };
        assert_eq!(chip_avatars(&[by_mail, u("b")], 2, true, &me(), on).shown(), &[1u16]);
        // Mine, but the service did not list me: not counted either.
        assert_eq!(chip_avatars(&[u("b")], 2, true, &me(), on).more, 0);
    }

    #[test]
    fn avatars_overlap_and_size_sets_the_width() {
        assert_eq!(avatars_width(0, 14.0, 0.0), 0.0);
        assert_eq!(avatars_width(1, 14.0, 0.0), 14.0);
        assert!((avatars_width(3, 14.0, 0.0) - (14.0 + 2.0 * 14.0 * 0.7)).abs() < 1e-4);
        assert!((avatars_width(3, 18.0, 0.0) - (18.0 + 2.0 * 18.0 * 0.7)).abs() < 1e-4);
        assert_eq!(avatars_width(1, 14.0, 10.0), 14.0 + GAP + 10.0);
        assert_eq!(avatars_width(0, 14.0, 10.0), 10.0);
    }
}
