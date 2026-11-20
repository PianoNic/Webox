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
            "ToggleAssistantPanel" => self.toast("Cisco AI Assistant: nicht verfügbar", now), // placeholder: Webox has no assistant panel
            "SearchInConversation" => { self.set_tab(Tab::Messaging); self.focus_search = true }
            "SignOut" => if self.client.is_some() { self.sc.sign_out_ask = true } else { self.toast("Sie sind nicht angemeldet.", now) },
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
            // "Home": the landing screen of Settings > Allgemein > Zielbildschirm (there is no separate home tab).
            "OpenHome" => self.set_tab(match self.settings["home"].as_str() { Some("Meetings") => Tab::Meetings, Some("Calling") => Tab::Calling, Some("Contacts") => Tab::Contacts, _ => Tab::Messaging }),
            "OpenDiagnostics" => crate::ui::menu::menu_open(ctx, crate::ui::app_dialogs::HEALTH), // the health checker is the diagnostics window
            "Minimize" | "Hide" => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            "Maximize" | "Zoom" => { let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false)); ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!m)) }
            "ShowParticipants" => if self.selected.is_some() { self.conv_tab = ConvTab::People },
            "ShowLinks" => if let Some(s) = self.selected.clone() { self.conv_tab = ConvTab::Content; crate::ui::space_tabs::set_content(ctx, &s, 1) },
            "SpaceSettings" | "ViewSpaceSettings" => if self.selected.is_some() && self.tab == Tab::Messaging { crate::ui::menu::menu_open(ctx, "gear") },
            "AddGif" => if self.selected.is_some() { self.gif_picker_toggle(ctx) },
            // Webox has no separate call dock window; its calling features are on the "Anrufe" page.
            "ShowHideCallingDock" | "CreateWhiteboard" => self.set_tab(if id == "CreateWhiteboard" { Tab::Whiteboards } else { Tab::Calling }),
            "OpenDialpad" => { self.set_tab(Tab::Calling); let mut s = self.pst(ctx); s.dial_open = true; self.set_pst(ctx, s); }
            _ => {}
        }
    }

    fn call_command(&mut self, id: &str) {
        let Some(c) = self.call.cur.as_ref().filter(|c| c.ended_at.is_none()) else { return };
        let mut a = CallActions::default();
        match (id, &c.state) {
            ("AnswerCall", CallState::Incoming) => a.answer = true,
            ("DeclineCall", CallState::Incoming) => a.hang = true,
            ("EndCall", s) if *s != CallState::Incoming => a.hang = true,
            ("ToggleAudioMute", CallState::Connected) => a.mute = true,
            _ => return,
        }
        self.apply_call_actions(a);
    }
}
