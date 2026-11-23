//! Type to focus (Webox addition, setting `type_to_composer`, default on): printable typing or Ctrl+V while no widget has the
//! keyboard focus moves the focus to the open chat's message box before any widget sees the keys, so the first character
//! lands there (at the stored caret, else at the end). It never sends: Enter in the same frame is ignored by the composer.
//! One check per frame, before the window is drawn (ui/frame.rs).
use crate::*;

/// The settings key ("Beim Tippen automatisch ins Nachrichtenfeld schreiben", Settings > Messaging).
pub(crate) const KEY: &str = "type_to_composer";
const COMPOSER: &str = "composer-edit"; // ui/composer/
const DRAWN: &str = "type-focus-drawn";
const PASSIVE: &str = "type-focus-passive";
const NOW: &str = "type-focus-now";

/// Everything outside the input events that decides whether keys may go to the composer.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Gate {
    pub enabled: bool,
    /// The open chat's composer was drawn last frame and accepts input (not read-only, not the New message pane).
    pub composer_ready: bool,
    pub window_focused: bool,
    /// A menu, dialog, popover, balloon or picker is open.
    pub popup_open: bool,
    /// Some widget holds the keyboard focus (a text field, a button, a list row).
    pub focused: bool,
    /// --screenshot run without a scripted type test.
    pub shot: bool,
}

/// Text that starts typing: printable, not only white space (a lone Space stays with whatever the user meant it for).
fn starts_typing(s: &str) -> bool { s.chars().any(|c| !c.is_whitespace() && !c.is_control()) }

/// Whether this frame's input should go to the composer. `image_paste` = Ctrl+V went down without a text paste (an image
/// on the clipboard; egui-winit sends no event for it). Ctrl / Cmd keep their shortcuts (egui-winit sends no text then);
/// Alt + letter or digit is a shortcut too, other Alt text is AltGr typing (@, {, EUR on German layouts).
pub(crate) fn decide(g: &Gate, events: &[egui::Event], mods: egui::Modifiers, image_paste: bool) -> bool {
    if !g.enabled || !g.composer_ready || !g.window_focused || g.popup_open || g.focused || g.shot { return false; }
    let paste = events.iter().any(|e| matches!(e, egui::Event::Paste(s) if !s.is_empty()));
    if paste || image_paste { return true; }
    if mods.command || mods.ctrl || mods.mac_cmd { return false; }
    events.iter().any(|e| match e {
        egui::Event::Text(s) => starts_typing(s) && !(mods.alt && s.chars().all(|c| c.is_ascii_alphanumeric())),
        egui::Event::Ime(egui::ImeEvent::Commit(s)) => starts_typing(s),
        _ => false,
    })
}

/// Overlays that are not popups (the message hover toolbar, upload strips) mark their layer each frame.
pub(crate) fn passive(ctx: &egui::Context, layer: egui::LayerId) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let v = d.get_temp_mut_or_default::<(u64, Vec<egui::LayerId>)>(egui::Id::new(PASSIVE));
        if v.0 != pass { *v = (pass, vec![]); }
        v.1.push(layer);
    });
}

/// Layers that are always there and not popups.
const STATIC_PASSIVE: [&str; 3] = ["list-splitter", "rail-edge", "local-share-card"];

/// A popup-like layer was visible last frame: anything Foreground or Debug that is not a known overlay. Middle (toasts,
/// edit overlay, banners) and Tooltip (hover hints) do not count; dialogs have a Foreground dim layer.
pub(crate) fn popup_layers_open(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    let marked = ctx.data(|d| d.get_temp::<(u64, Vec<egui::LayerId>)>(egui::Id::new(PASSIVE))).filter(|v| v.0 + 1 >= pass).map(|v| v.1).unwrap_or_default();
    ctx.memory(|m| m.areas().visible_layer_ids()).into_iter().any(|l| {
        matches!(l.order, egui::Order::Foreground | egui::Order::Debug)
            && !marked.contains(&l) && !STATIC_PASSIVE.iter().any(|n| l.id == egui::Id::new(*n))
    })
}

/// The composer marks the space it drew (ui/composer/draw.rs).
pub(crate) fn composer_drawn(ctx: &egui::Context, space: &str) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DRAWN), (pass, space.to_string())));
}
fn drawn_last_frame(ctx: &egui::Context) -> Option<String> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::new(DRAWN))).filter(|(p, _)| p + 2 >= pass).map(|(_, s)| s)
}

/// The focus moved to the composer this frame: it must not send on an Enter of the same frame.
pub(crate) fn redirected_now(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<u64>(egui::Id::new(NOW))) == Some(ctx.cumulative_pass_nr())
}

/// Moves the focus to the composer when `decide` says so. Returns whether it did.
pub(crate) fn route(ctx: &egui::Context, g: &Gate, image_paste: bool) -> bool {
    let go = ctx.input(|i| decide(g, &i.events, i.modifiers, image_paste));
    if go {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(COMPOSER)));
        let pass = ctx.cumulative_pass_nr(); // outside the closure: the context is locked inside it
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(NOW), pass));
    }
    go
}

impl App {
    /// Called once per frame before anything is drawn.
    pub(crate) fn type_focus_frame(&mut self, ctx: &egui::Context) {
        // --screenshot with WEBOX_SHOT_TYPEFOCUS=1: scripted typing (WEBOX_SHOT_KEY=60:text=H) in a window that may be in the background.
        let scripted = self.shot.is_some() && std::env::var_os("WEBOX_SHOT_TYPEFOCUS").is_some();
        let window_focused = scripted || ctx.input(|i| i.viewport().focused) != Some(false);
        let image_paste = super::attach::paste_key_poll(ctx, window_focused)
            && !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        let chat_tab = self.tab == Tab::Messaging || self.tab == Tab::Teams && self.team_sel.is_some();
        let drawn = drawn_last_frame(ctx);
        let composer_ready = !self.need_login && chat_tab && !super::new_message::is_open(ctx)
            && drawn.is_some_and(|d| d != super::new_message::DRAFT_ID && self.selected.as_deref() == Some(d.as_str()));
        let popup_open = popup_layers_open(ctx) || self.rename_input.is_some() || self.confirm_leave.is_some() || self.new_space.is_some()
            || self.show_shortcuts || super::invite::is_open(ctx) || super::share_picker::is_open() || super::gif_picker::gif_picker_open()
            || ctx.memory(|m| m.any_popup_open());
        let g = Gate {
            enabled: self.settings[KEY].as_bool().unwrap_or(true), composer_ready, window_focused, popup_open,
            focused: ctx.memory(|m| m.focused().is_some()),
            shot: self.shot.is_some() && !scripted,
        };
        route(ctx, &g, image_paste);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open() -> Gate { Gate { enabled: true, composer_ready: true, window_focused: true, ..Default::default() } }
    fn text(s: &str) -> Vec<egui::Event> { vec![egui::Event::Text(s.into())] }
    const NONE: egui::Modifiers = egui::Modifiers::NONE;

    #[test]
    fn printable_with_nothing_focused_redirects() {
        assert!(decide(&open(), &text("h"), NONE, false));
        assert!(decide(&open(), &text("H"), egui::Modifiers::SHIFT, false), "Shift is typing");
        assert!(decide(&open(), &[egui::Event::Ime(egui::ImeEvent::Commit("日".into()))], NONE, false));
        assert!(decide(&open(), &text("@"), egui::Modifiers::ALT, false), "AltGr text on German layouts");
    }

    #[test]
    fn blocked_states_never_redirect() {
        let ev = text("h");
        for g in [Gate { focused: true, ..open() }, Gate { popup_open: true, ..open() }, Gate { enabled: false, ..open() },
                  Gate { composer_ready: false, ..open() }, Gate { window_focused: false, ..open() }, Gate { shot: true, ..open() }] {
            assert!(!decide(&g, &ev, NONE, false), "{g:?}");
            assert!(!decide(&g, &[egui::Event::Paste("x".into())], egui::Modifiers::COMMAND, false), "{g:?}");
            assert!(!decide(&g, &[], egui::Modifiers::COMMAND, true), "{g:?}");
        }
    }

    #[test]
    fn shortcuts_and_non_text_keys_stay() {
        assert!(!decide(&open(), &text("k"), egui::Modifiers::COMMAND, false));
        assert!(!decide(&open(), &text("k"), egui::Modifiers::ALT, false), "Alt + letter is a shortcut");
        assert!(!decide(&open(), &text(" "), NONE, false), "a lone Space starts nothing");
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: NONE };
        for k in [egui::Key::Enter, egui::Key::Escape, egui::Key::Tab, egui::Key::ArrowDown, egui::Key::PageUp, egui::Key::Home, egui::Key::F5, egui::Key::Space] {
            assert!(!decide(&open(), &[key(k)], NONE, false), "{k:?}");
        }
    }

    #[test]
    fn paste_with_nothing_focused_redirects() {
        assert!(decide(&open(), &[egui::Event::Paste("hallo".into())], egui::Modifiers::COMMAND, false));
        assert!(decide(&open(), &[], egui::Modifiers::COMMAND, true), "image on the clipboard");
    }

    /// The first character is not lost: it lands in the composer in the frame that moves the focus, the next one follows.
    #[test]
    fn first_character_lands_in_the_composer() {
        let ctx = egui::Context::default();
        let mut draft = String::from("Hi ");
        let frame = |events: Vec<egui::Event>, draft: &mut String| {
            let input = egui::RawInput { events, ..Default::default() };
            let mut routed = false;
            let _ = ctx.run(input, |ctx| {
                routed = route(ctx, &Gate { focused: ctx.memory(|m| m.focused().is_some()), ..open() }, false);
                egui::CentralPanel::default().show(ctx, |ui| { ui.add(egui::TextEdit::multiline(draft).id(egui::Id::new(COMPOSER))); });
            });
            routed
        };
        assert!(!frame(vec![], &mut draft));
        assert!(frame(text("d"), &mut draft), "nothing focused: the focus moves");
        assert_eq!(draft, "Hi d");
        assert!(!frame(text("u"), &mut draft), "the composer has the focus now: egui handles the key");
        assert_eq!(draft, "Hi du");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new(COMPOSER))));
    }

    #[test]
    fn a_focused_field_keeps_its_keys() {
        let ctx = egui::Context::default();
        let (mut draft, mut search) = (String::new(), String::new());
        let search_id = egui::Id::new("some-search");
        for (i, ev) in [vec![], text("a")].into_iter().enumerate() {
            let _ = ctx.run(egui::RawInput { events: ev, ..Default::default() }, |ctx| {
                if i == 0 { ctx.memory_mut(|m| m.request_focus(search_id)); }
                route(ctx, &Gate { focused: ctx.memory(|m| m.focused().is_some()), ..open() }, false);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut search).id(search_id));
                    ui.add(egui::TextEdit::multiline(&mut draft).id(egui::Id::new(COMPOSER)));
                });
            });
        }
        assert_eq!((search.as_str(), draft.as_str()), ("a", ""));
    }
}
