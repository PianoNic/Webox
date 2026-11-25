//! Commands implemented by the shortcut engine itself (`table::Kind::Cmd`): the ones with no existing `Act`.
//! Call commands only act on a call that is already on screen (answer / decline / end / mute); no call is ever placed here.
use crate::cloud::locus::CallState;
use crate::ui::call::CallActions;
use crate::*;

impl App {
    pub(crate) fn run_cmd(&mut self, id: &str, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let composer = egui::Id::new("composer-edit"); // ui/conversation.rs
        match id {
            "OpenHelpCenter" => system::open(LEARN_MORE_URL),
            "ToggleAssistantPanel" => self.toast(format!("Cisco AI Assistant: {}", crate::trq!("unavailable")), now), // placeholder: Webox has no assistant panel
            "SearchInConversation" => { self.set_tab(Tab::Messaging); self.focus_search = true }
            "SignOut" => if self.client.is_some() { self.sc.sign_out_ask = true } else { self.toast(crate::trq!("You are not signed in."), now) },
            "ShowAll" => { self.set_tab(Tab::Messaging); self.list_tab = ListTab::All; self.filter = Filter::None }
            // The open list jumps to its newest message (ui/messages/jump_button.rs).
            "JumpToBottom" => if self.selected.is_some() { self.conv_tab = ConvTab::Messages; crate::ui::messages::request_jump_to_bottom(ctx) },
            "ShowMessages" => if self.selected.is_some() { self.conv_tab = ConvTab::Messages },
            "ShowSpaceContent" => if self.selected.is_some() { self.conv_tab = ConvTab::Content },
            "GoToTeam" => self.set_tab(Tab::Teams),
            "CollapseSections" => { self.collapsed.insert("fav"); self.collapsed.insert("other"); }
            "ExpandSections" => self.collapsed.clear(),
            "AddEmoji" => if self.selected.is_some() { self.emoji_picker_toggle(ctx) },
            "FocusMainInputBox" => ctx.memory_mut(|m| m.request_focus(composer)),
            "NewMessageButton" => { self.compose = Compose::New; ctx.memory_mut(|m| m.request_focus(composer)) }
            "NewReplyButton" => ctx.memory_mut(|m| m.request_focus(composer)),
            "FocusMainConversation" => ctx.memory_mut(|m| if let Some(f) = m.focused() { m.surrender_focus(f) }),
            "AnswerCall" | "DeclineCall" | "EndCall" | "ToggleAudioMute" => self.call_command(id),
            // Ctrl+Shift+D / Ctrl+Shift+Z: the share picker / stop sharing (what the call window's share button does)
            "ShareContent" | "StopSharing" => if self.call_cmd_applies(id, ctx) { self.share_button(now) },
            // Alt+Enter (F11 in the call window as well): the call window in full screen, or back
            "FullScreen" => if self.call_cmd_applies(id, ctx) { self.call.win.full_toggle = true },
            // "Home": the landing screen of Settings > General > start screen (there is no separate home tab).
            "OpenHome" => self.set_tab(match self.settings["home"].as_str() { Some("Meetings") => Tab::Meetings, Some("Calling") => Tab::Calling, Some("Contacts") => Tab::Contacts, _ => Tab::Messaging }),
            "OpenDiagnostics" => crate::ui::menu::menu_open(ctx, crate::ui::app_dialogs::HEALTH), // the health checker is the diagnostics window
            "Minimize" | "Hide" => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            "Maximize" | "Zoom" => { let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false)); ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!m)) }
            "ShowParticipants" => if self.selected.is_some() { self.conv_tab = ConvTab::People },
            "ShowLinks" => if let Some(s) = self.selected.clone() { self.conv_tab = ConvTab::Content; crate::ui::space_tabs::set_content(ctx, &s, 1) },
            "SpaceSettings" | "ViewSpaceSettings" => if self.selected.is_some() && self.tab == Tab::Messaging { crate::ui::menu::menu_open(ctx, "gear") },
            "AddGif" => if self.selected.is_some() { self.gif_picker_toggle(ctx) },
            // Ctrl+Shift+X: shows / hides the calling dock window (ui/call_win/dock.rs); without separate call windows the
            // calling features are on the "Calling" page.
            "ShowHideCallingDock" => if !self.toggle_calling_dock(ctx) { self.set_tab(Tab::Calling) },
            "CreateWhiteboard" => self.set_tab(Tab::Whiteboards),
            "OpenDialpad" => { self.set_tab(Tab::Calling); let mut s = self.pst(ctx); s.dial_open = true; self.set_pst(ctx, s); }
            _ => {}
        }
    }

    fn call_command(&mut self, id: &str) {
        if let Some(a) = call_actions(id, self.live_call_state()) { self.apply_call_actions(a); }
    }

    /// Call command `id` does something now; otherwise its key is left to the rest of the app.
    pub(super) fn call_cmd_applies(&self, id: &str, ctx: &egui::Context) -> bool {
        let state = self.live_call_state();
        call_actions(id, state).is_some() || call_view_cmd(id, state, self.call.sharing, self.call.share_pending, self.separate_call_windows(ctx))
    }

    /// Keys pressed while the call window has the focus (the main window's dispatch sees only its own): the registry's call
    /// commands, with the user's bindings.
    pub(crate) fn call_window_shortcuts(&mut self, vctx: &egui::Context) {
        let reg = super::registry();
        let events: Vec<(egui::Key, Option<egui::Key>, egui::Modifiers)> = vctx.input(|i| i.events.iter().filter_map(|e| match e {
            egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } => Some((*key, *physical_key, *modifiers)), _ => None }).collect());
        for (key, phys, mods) in events {
            let hit = (0..reg.cmds.len()).find(|&i| {
                let c = &reg.cmds[i];
                #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
                if self.sc.global.is_registered(i) { return false; } // the hotkey thread delivers it
                c.needs_call() && self.sc.res[i].key.is_some_and(|k| k.matches_at(key, phys, &mods, false)) && self.call_cmd_applies(c.id, vctx)
            });
            let Some(i) = hit else { continue };
            vctx.input_mut(|inp| {
                let pos = inp.events.iter().position(|e| matches!(e, egui::Event::Key { key: k2, physical_key: p2, pressed: true, modifiers: m2, .. } if *k2 == key && *p2 == phys && *m2 == mods));
                if let Some(p) = pos { super::take_key(&mut inp.events, p); }
            });
            self.run_command(i, vctx);
        }
    }

    /// The state of the call on screen, if one is still going.
    pub(super) fn live_call_state(&self) -> Option<&CallState> { self.call.cur.as_ref().filter(|c| c.ended_at.is_none()).map(|c| &c.state) }
}

/// The call window's commands beyond `call_actions`: share (connected, not sharing yet), stop sharing (while sharing) and
/// full screen (a call window is on screen: separate call windows, not ringing).
pub(super) fn call_view_cmd(id: &str, state: Option<&CallState>, sharing: bool, share_pending: bool, call_window: bool) -> bool {
    match (id, state) {
        ("ShareContent", Some(CallState::Connected)) => !sharing && !share_pending,
        ("StopSharing", Some(CallState::Connected)) => sharing,
        ("FullScreen", Some(s)) => call_window && *s != CallState::Incoming,
        _ => false,
    }
}

/// What call command `id` does to a call in `state`; `None` when it does nothing (no call, or not the right moment), so its key
/// is left to the rest of the app (Ctrl+Shift+C copies unless a call is ringing).
pub(super) fn call_actions(id: &str, state: Option<&CallState>) -> Option<CallActions> {
    let mut a = CallActions::default();
    match (id, state?) {
        ("AnswerCall", CallState::Incoming) => a.answer = true,
        ("DeclineCall", CallState::Incoming) => a.hang = true,
        ("EndCall", s) if *s != CallState::Incoming => a.hang = true,
        ("ToggleAudioMute", CallState::Connected) => a.mute = true,
        _ => return None,
    }
    Some(a)
}
