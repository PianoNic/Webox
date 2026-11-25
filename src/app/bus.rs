//! The action bus and the state keys (docs/theming-engine.md, "Behaviour and coherence" §3).
//!
//! Every change the UI makes to app state goes through one entry point, [`App::dispatch`], whatever started it: a shortcut, a
//! built-in button, later a plugin (`ui.dispatch`) or a theme's widget. Each action has a stable string id ("space.open") and
//! the same guards for every caller, so a button a layout moves elsewhere, or a plugin's button, behaves exactly like the
//! built-in one. A refused action shows the same toast as the built-in path.
//!
//! State keys are the read-only, typed view of app state that layout conditions and theme bindings use ("nav.page",
//! "space.selected", "call.active", ...). [`StateBits`] packs the ones layouts switch on into one `u64` per frame, so a
//! layout re-solves only when a bit changes. The id lists are published in docs/plugins.md and stay unstable until the
//! theme packages ship (theming engine phase 7).
use crate::*;

/// Who dispatched an action: for logs and for rules such as "themes act only on a user gesture".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Theme: dispatch from theme packages' widgets (theming phase 8)
pub(crate) enum Origin { Shortcut, Ui, Plugin, Theme }

/// Why an action was refused; shown as a toast and answered to plugins.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Refusal(pub(crate) String);

/// Panels a layout can open and close (`panel.toggle`). The built-in layout has one: the space list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Panel { List }

#[allow(dead_code)]
impl Panel {
    pub(crate) fn id(self) -> &'static str { match self { Panel::List => "list" } }
    pub(crate) fn parse(s: &str) -> Option<Panel> { match s { "list" => Some(Panel::List), _ => None } }
}

fn tab_id(t: Tab) -> &'static str {
    match t {
        Tab::Messaging => "messaging", Tab::Meetings => "meetings", Tab::Calling => "calls", Tab::Whiteboards => "whiteboards",
        Tab::Contacts => "contacts", Tab::Teams => "teams", Tab::Voicemail => "voicemail", Tab::Settings => "settings",
    }
}
fn tab_parse(s: &str) -> Option<Tab> {
    Some(match s {
        "messaging" => Tab::Messaging, "meetings" => Tab::Meetings, "calls" => Tab::Calling, "whiteboards" => Tab::Whiteboards,
        "contacts" => Tab::Contacts, "teams" => Tab::Teams, "voicemail" => Tab::Voicemail, "settings" => Tab::Settings, _ => return None,
    })
}

impl Act {
    /// The action's stable id (docs/plugins.md, "Actions").
    pub(crate) fn id(&self) -> &'static str {
        match self {
            Act::Tab(_) => "nav.page", Act::Back => "nav.back", Act::Forward => "nav.forward", Act::Prev => "nav.prev_space",
            Act::Next => "nav.next_space", Act::Search => "search.open", Act::Prefs => "settings.open", Act::Shortcuts => "shortcuts.toggle",
            Act::CreateSpace => "space.new", Act::Contact => "contacts.open", Act::Quit => "app.quit", Act::ZoomIn => "app.zoom_in",
            Act::ZoomOut => "app.zoom_out", Act::List(_) => "list.tab", Act::Filter(_) => "list.filter", Act::Favorite => "space.favorite_toggle",
            Act::ToggleRail => "ui.rail_wide", Act::AddPeople => "space.add_people", Act::LeaveSpace => "space.leave", Act::Reload => "space.reload",
            Act::CopyInfo => "space.copy_info", Act::Composer(_) => "composer.command", Act::SpaceOpen(_) => "space.open",
            Act::SpaceFavorite(..) => "space.favorite", Act::CallStart { .. } => "call.start", Act::CallHangUp => "call.hang_up",
            Act::CallMute => "call.mute", Act::CallVideo => "call.video", Act::PanelToggle(_) => "panel.toggle",
        }
    }

    /// Builds an action from its id and JSON arguments (plugins, theme widgets). `None` for an unknown id or missing arguments.
    pub(crate) fn parse(id: &str, args: &serde_json::Value) -> Option<Act> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str());
        let b = |k: &str| args.get(k).and_then(|v| v.as_bool());
        Some(match id {
            "nav.page" => Act::Tab(tab_parse(s("page")?)?),
            "nav.back" => Act::Back, "nav.forward" => Act::Forward, "nav.prev_space" => Act::Prev, "nav.next_space" => Act::Next,
            "search.open" => Act::Search, "settings.open" => Act::Prefs, "shortcuts.toggle" => Act::Shortcuts, "space.new" => Act::CreateSpace,
            "contacts.open" => Act::Contact, "app.zoom_in" => Act::ZoomIn, "app.zoom_out" => Act::ZoomOut,
            "list.tab" => Act::List(match s("tab")? { "all" => ListTab::All, "direct" => ListTab::Direct, "spaces" => ListTab::Spaces, "public" => ListTab::Public, _ => return None }),
            "space.favorite_toggle" => Act::Favorite, "ui.rail_wide" => Act::ToggleRail, "space.add_people" => Act::AddPeople,
            "space.leave" => Act::LeaveSpace, "space.reload" => Act::Reload, "space.copy_info" => Act::CopyInfo,
            "space.open" => Act::SpaceOpen(s("space")?.to_string()),
            "space.favorite" => Act::SpaceFavorite(s("space")?.to_string(), b("on")?),
            "call.start" => Act::CallStart { email: s("email")?.to_string(), name: s("name").unwrap_or_default().to_string(), video: b("video").unwrap_or(false) },
            "call.hang_up" => Act::CallHangUp, "call.mute" => Act::CallMute, "call.video" => Act::CallVideo,
            "panel.toggle" => Act::PanelToggle(Panel::parse(s("panel")?)?),
            // app.quit, list.filter and composer.command stay built-in: quitting and formatting are the user's own keys.
            _ => return None,
        })
    }
}

#[allow(dead_code)] // state() and state_bits() serve the layout engine (theming phase 4) and theme bindings
impl App {
    fn call_active(&self) -> bool { self.call.cur.as_ref().is_some_and(|c| c.ended_at.is_none()) }

    /// The one way UI changes app state. Checks the same guards for every caller and shows a refusal as a toast.
    pub(crate) fn dispatch(&mut self, a: Act, origin: Origin, ctx: &egui::Context) -> Result<(), Refusal> {
        let r = self.guard(&a);
        if let Err(Refusal(why)) = &r {
            let now = ctx.input(|i| i.time);
            if !why.is_empty() { self.toast(why.clone(), now); }
            // A plugin's refused action also goes to the plugin log, by its id, so its author can see why.
            if origin == Origin::Plugin { self.plugin_log.push(format!("ui.dispatch {} refused: {}", a.id(), if why.is_empty() { "nothing to act on" } else { why })); }
            return r;
        }
        self.run(a, ctx);
        Ok(())
    }

    /// The guards of an action: what must hold before it runs.
    fn guard(&self, a: &Act) -> Result<(), Refusal> {
        let no = |s: &str| Err(Refusal(tr(s).to_string()));
        match a {
            Act::SpaceOpen(id) | Act::SpaceFavorite(id, _) if !self.spaces.iter().any(|s| &s.id == id) => no("This space is not available."),
            Act::CallStart { email, .. } => {
                if self.call_active() { return no("A call is already in progress."); }
                if let Err(e) = cloud::locus::call_allowed(email, std::env::var(cloud::locus::GUARD_ENV).ok().as_deref()) { return Err(Refusal(e)); }
                if self.client.is_none() { return no("Not connected"); }
                Ok(())
            }
            // No call: nothing to do, and nothing to say.
            Act::CallHangUp | Act::CallMute | Act::CallVideo if !self.call_active() => Err(Refusal(String::new())),
            _ => Ok(()),
        }
    }

    /// Runs an action whose guards passed.
    fn run(&mut self, a: Act, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        match a {
            Act::SpaceOpen(id) => self.open_space(id),
            Act::SpaceFavorite(id, on) => self.set_favorite(&id, on),
            Act::CallStart { email, name, video } => self.start_call_with(&email, &name, now, video),
            Act::CallHangUp => self.hang_up(),
            Act::CallMute => self.apply_call_actions(crate::ui::call::CallActions { mute: true, ..Default::default() }),
            Act::CallVideo => self.apply_call_actions(crate::ui::call::CallActions { video: true, ..Default::default() }),
            Act::PanelToggle(Panel::List) => self.list_hidden = !self.list_hidden,
            other => self.act_builtin(other, ctx),
        }
    }

    /// A state key's current value (docs/plugins.md, "State keys"). `None` for an unknown key.
    pub(crate) fn state(&self, key: StateKey) -> StateVal<'_> {
        let sel = self.selected.as_deref().and_then(|id| self.spaces.iter().find(|s| s.id == id));
        match key {
            StateKey::NavPage => StateVal::Str(tab_id(self.tab)),
            StateKey::SpaceSelected => sel.map_or(StateVal::None, |s| StateVal::Str(&s.id)),
            StateKey::SpaceSelectedDirect => StateVal::Bool(sel.is_some_and(|s| s.direct)),
            StateKey::SpaceSelectedClosed => StateVal::Bool(sel.is_some_and(|s| s.closed)),
            StateKey::CallActive => StateVal::Bool(self.call_active()),
            StateKey::UiRailWide => StateVal::Bool(self.rail_wide),
            StateKey::UiListHidden => StateVal::Bool(self.list_hidden),
            StateKey::UnreadCount => StateVal::Num(self.spaces.iter().filter(|s| s.unread).count() as f64),
        }
    }

    /// The layout-relevant state of this frame, packed (see [`StateBits`]). `width` is the window's content width in points.
    pub(crate) fn state_bits(&self, width: f32) -> StateBits {
        let sel = self.selected.as_deref().and_then(|id| self.spaces.iter().find(|s| s.id == id));
        let mut b = self.tab as u64 & 0xF;
        b |= (WidthClass::of(width) as u64) << 4;
        let flag = |on: bool, bit: u32| if on { 1u64 << bit } else { 0 };
        b |= flag(sel.is_some(), 8) | flag(sel.is_some_and(|s| s.direct), 9) | flag(sel.is_some_and(|s| s.closed), 10)
            | flag(self.call_active(), 11) | flag(self.rail_wide, 12) | flag(self.list_hidden, 13);
        StateBits(b)
    }
}

/// The state keys (docs/plugins.md, "State keys").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(crate) enum StateKey { NavPage, SpaceSelected, SpaceSelectedDirect, SpaceSelectedClosed, CallActive, UiRailWide, UiListHidden, UnreadCount }

#[allow(dead_code)]
impl StateKey {
    pub(crate) const ALL: [StateKey; 8] = [StateKey::NavPage, StateKey::SpaceSelected, StateKey::SpaceSelectedDirect, StateKey::SpaceSelectedClosed,
        StateKey::CallActive, StateKey::UiRailWide, StateKey::UiListHidden, StateKey::UnreadCount];
    pub(crate) fn id(self) -> &'static str {
        match self {
            StateKey::NavPage => "nav.page", StateKey::SpaceSelected => "space.selected", StateKey::SpaceSelectedDirect => "space.selected.direct",
            StateKey::SpaceSelectedClosed => "space.selected.closed", StateKey::CallActive => "call.active", StateKey::UiRailWide => "ui.rail_wide",
            StateKey::UiListHidden => "ui.list_hidden", StateKey::UnreadCount => "unread.count",
        }
    }
    pub(crate) fn parse(s: &str) -> Option<StateKey> { Self::ALL.into_iter().find(|k| k.id() == s) }
}

/// A state key's value.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(dead_code)]
pub(crate) enum StateVal<'a> { None, Bool(bool), Num(f64), Str(&'a str) }

/// Window width classes (`window.width_class`): narrow < 720 ≤ medium < 1100 ≤ wide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(crate) enum WidthClass { Narrow, Medium, Wide }

#[allow(dead_code)]
impl WidthClass {
    pub(crate) fn of(w: f32) -> WidthClass { if w < 720.0 { WidthClass::Narrow } else if w < 1100.0 { WidthClass::Medium } else { WidthClass::Wide } }
}

/// The state layouts switch on, packed: bits 0-3 the page, 4-5 the width class, 8 a space selected, 9 it is direct, 10 it
/// is closed, 11 a call is active, 12 the rail is wide, 13 the list is hidden. Equal bits mean an equal layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[allow(dead_code)]
pub(crate) struct StateBits(pub(crate) u64);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_round_trip_through_parse() {
        let cases = [
            (Act::Tab(Tab::Contacts), json!({"page": "contacts"})), (Act::Back, json!({})), (Act::Forward, json!({})),
            (Act::Prev, json!({})), (Act::Next, json!({})), (Act::Search, json!({})), (Act::Prefs, json!({})), (Act::CreateSpace, json!({})),
            (Act::ToggleRail, json!({})), (Act::SpaceOpen("s1".into()), json!({"space": "s1"})),
            (Act::SpaceFavorite("s1".into(), true), json!({"space": "s1", "on": true})),
            (Act::CallStart { email: "a@b.c".into(), name: "A".into(), video: true }, json!({"email": "a@b.c", "name": "A", "video": true})),
            (Act::CallHangUp, json!({})), (Act::CallMute, json!({})), (Act::CallVideo, json!({})),
            (Act::PanelToggle(Panel::List), json!({"panel": "list"})), (Act::List(ListTab::Direct), json!({"tab": "direct"})),
        ];
        for (a, args) in cases { assert_eq!(Act::parse(a.id(), &args).as_ref(), Some(&a), "{}", a.id()); }
    }

    #[test]
    fn parse_refuses_unknown_ids_missing_arguments_and_built_in_only_actions() {
        assert_eq!(Act::parse("space.explode", &json!({})), None);
        assert_eq!(Act::parse("space.open", &json!({})), None);
        assert_eq!(Act::parse("nav.page", &json!({"page": "nowhere"})), None);
        assert_eq!(Act::parse("panel.toggle", &json!({"panel": "members"})), None);
        for id in ["app.quit", "list.filter", "composer.command"] { assert_eq!(Act::parse(id, &json!({})), None, "{id}"); }
    }

    #[test]
    fn every_page_id_parses_back() {
        for t in [Tab::Messaging, Tab::Meetings, Tab::Calling, Tab::Whiteboards, Tab::Contacts, Tab::Teams, Tab::Voicemail, Tab::Settings] {
            assert!(tab_parse(tab_id(t)) == Some(t));
        }
    }

    #[test]
    fn state_keys_and_width_classes() {
        for k in StateKey::ALL { assert_eq!(StateKey::parse(k.id()), Some(k)); }
        assert_eq!(StateKey::parse("nope"), None);
        assert_eq!(WidthClass::of(719.9), WidthClass::Narrow);
        assert_eq!(WidthClass::of(720.0), WidthClass::Medium);
        assert_eq!(WidthClass::of(1100.0), WidthClass::Wide);
    }
}
