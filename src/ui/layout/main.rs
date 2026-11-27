//! The main window drawn through the layout engine. The active layout is solved once per change (window area, layout
//! state bits, named lengths); the solved rects then decide which components draw and give their panels their sizes and
//! sides. The components still live in egui panels, so `ctx.available_rect()` and every popup, menu and overlay that
//! relies on it keep working as before.
use super::{load, Compiled, Component, Env, KeyVal, Placed, DEFAULT_LAYOUT};
use crate::app::bus::{StateKey, StateVal};
use eframe::egui::Rect;

/// Named lengths the main window provides (`{metrics.header_h}`, ...). Others resolve to 0.
const LENS: [&str; 4] = ["metrics.header_h", "rail.width", "list.width", "settings.nav_width"];

/// What a solve depends on beyond the area and the state bits.
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) struct Inputs { pub header_h: f32, pub rail_w: f32, pub list_w: f32, pub settings_nav_w: f32 }

pub(crate) struct Main {
    compiled: Compiled,
    /// Index into LENS per compiled length name.
    lens: Vec<Option<usize>>,
    out: Vec<Placed>,
    key: Option<(Rect, u64, [u32; 4])>,
}

impl Default for Main {
    fn default() -> Self {
        let (_, compiled) = load(DEFAULT_LAYOUT).expect("the built-in layout is valid (ui/layout/tests.rs)");
        Main::new(compiled)
    }
}

/// What the main window reads per state key, by the compiled key's name.
pub(crate) trait Keys { fn key(&self, name: &str) -> KeyVal<'_>; }

struct E<'a, K: Keys> { m: &'a Compiled, lens: &'a [Option<usize>], k: &'a K, i: Inputs }
impl<K: Keys> Env for E<'_, K> {
    fn key(&self, k: usize) -> KeyVal<'_> { self.k.key(&self.m.keys[k]) }
    fn len(&self, n: usize) -> f32 {
        match self.lens[n] { Some(0) => self.i.header_h, Some(1) => self.i.rail_w, Some(2) => self.i.list_w, Some(3) => self.i.settings_nav_w, _ => 0.0 }
    }
}

impl Main {
    pub(crate) fn new(compiled: Compiled) -> Main {
        let lens = compiled.lens.iter().map(|n| LENS.iter().position(|l| l == n)).collect();
        Main { compiled, lens, out: Vec::with_capacity(16), key: None }
    }

    /// Solves for this frame unless nothing it depends on changed. `bits` must change whenever a key the layout reads does.
    pub(crate) fn update(&mut self, area: Rect, bits: u64, i: Inputs, keys: &impl Keys) {
        let key = (area, bits, [i.header_h.to_bits(), i.rail_w.to_bits(), i.list_w.to_bits(), i.settings_nav_w.to_bits()]);
        if self.key == Some(key) { return; }
        self.key = Some(key);
        let env = E { m: &self.compiled, lens: &self.lens, k: keys, i };
        let mut out = std::mem::take(&mut self.out);
        self.compiled.solve(area, &env, &mut out);
        self.out = out;
    }

    /// Where the last solve placed `c`, if it is shown.
    pub(crate) fn rect(&self, c: Component) -> Option<Rect> {
        self.out.iter().find_map(|p| match p { Placed::Component(x, r) if *x == c => Some(*r), _ => None })
    }

    /// The area of the last solve.
    pub(crate) fn area(&self) -> Option<Rect> { self.key.map(|k| k.0) }

    /// Whether `c` sits at the right edge of the solved area (a rail docked right).
    pub(crate) fn at_right(&self, c: Component) -> bool {
        match (self.rect(c), self.area()) { (Some(r), Some(a)) => r.left() > a.left() && (r.right() - a.right()).abs() < 0.5, _ => false }
    }
}

impl Main {
    /// A stand-in without nodes while the real one is borrowed for a solve (allocates nothing).
    fn placeholder() -> Main { Main { compiled: Compiled { nodes: vec![], keys: vec![], lens: vec![] }, lens: vec![], out: vec![], key: None } }
}

impl crate::App {
    /// Solves the main window's layout for this frame, over what the window edges leave (call after `edge_strips`).
    pub(crate) fn solve_layout(&mut self, ctx: &eframe::egui::Context) {
        let area = ctx.available_rect();
        let i = Inputs { header_h: crate::ui::chrome::header_h(), rail_w: self.rail_width(ctx), list_w: self.list_card_width(ctx),
            settings_nav_w: crate::ui::settings::NAV_W };
        let bits = self.state_bits(area.width()).0 | (u64::from(self.team_sel.is_some()) << 14);
        let mut m = std::mem::replace(&mut self.layout, Main::placeholder());
        m.update(area, bits, i, self);
        self.layout = m;
    }
}

impl Keys for crate::App {
    fn key(&self, name: &str) -> KeyVal<'_> {
        // A team's spaces show the messaging arrangement (the Teams page with a team open).
        if name == "nav.page" && self.tab == crate::Tab::Teams && self.team_sel.is_some() { return KeyVal::Str("messaging") }
        match StateKey::parse(name).map(|k| self.state(k)) {
            Some(StateVal::Bool(b)) => KeyVal::Bool(b),
            Some(StateVal::Str(s)) => KeyVal::Str(s),
            _ => KeyVal::None,
        }
    }
}
