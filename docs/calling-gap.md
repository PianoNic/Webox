# Calling: official Webex desktop app compared with Webox

Read-only inventory, 2026-11-24. Webox is at `main` 2a9cd2a (PR #20, screen share sending). Receiving a share is being built in parallel and is marked **in progress** below.

## Evidence base and conventions

**Official app (Webex 46.11.0.36194, Windows)**
* Extracted QML: `checks/qmlsrc/WebexQml/`. The call-relevant folders are:
  * `CallWindowDialog/`
  * `PhoneWidget/` (the "calling dock")
  * `Calling/`
  * `MeetingViewLayout/`
  * `SwitchAudioView/`
  * `EscalationInviteAndMeet/`
  * `Voicemail/`
  * `Settings/Calling/`, `Settings/Notifications/`, `Settings/Settings*Page.qml`
  * `ParticipantList/`, `Sharing/`, `CaptionsHighLights/`
* Language pack: `assets/cisco/i18n/index_en_de.tsv`. Strings are quoted as "…" with their context where it is not `QObject`.
* Strings from `spark-windows-desktop-ui.dll`, quoted as `dll:`. These include:
  * the telephony operation names `[holdCall] [resumeCall] [transferCall] [directTransferCall] [mergeCall] [parkCall] [resumeCallPark] [escalateCall] [deescalateCall] [moveMedia] [requestFloor] …`;
  * the operation/metric ids `call_hold call_resume call_transfer call_transfer_type_consult call_transfer_type_now call_merge call_park call_resumePark call_pickup call_pull call_retrieve call_move call_addParticipant call_addGuest call_addPSTNGuest call_addSipDeviceGuest call_removeParticipant call_admitParticipant call_sendDtmf call_setMeetingLayout call_recordMeeting call_toggleTranscription call_toggleMeetingReaction call_lockMeeting call_floorRequest call_keepAlive call_reconnect …`;
  * feature toggles such as `desktop-locus-hold-resume-enabled`, `desktop-join-meeting-in-call-enabled`, `desktop-move-call-notification-enabled-ga` and `desktop-broadworks-*`.
* Earlier notes: `checks/call-flow-spec.md` (locus/ROAP flow of a 1:1 call) and `checks/protocol_coverage.md`.
* Beyond this, **the control-bar button list is not in QML.** `CallControlBar.qml` is a generic responsive container fed by a C++ model (`CallControlBarItemModel`, `CallControlMoreMenuManager`, `CallControlRaiseHandReactionManager`, `CallControlRecordManager`, `CallControlUCMAndBWCManager`, …). Which buttons appear in which call type therefore comes from strings and manager names, not from a QML list. Where that leaves a doubt, the row says so.

**Webox:** the code was read on `main` (call files listed in each row).

**Status values**
* **done**: works end to end.
* **partial**: a part exists, or it only works locally.
* **missing**
* **in progress**: another agent is building it now.

**Needs**
* **UI**: UI only.
* **locus**: Locus REST / mercury.
* **media**: webrtc-rs, capture or codec work.
* **WxC**: a Webex Calling backend (Mobius / BroadWorks), licence-gated (see section 3).
* **meet**: meetingInfo / meetings API.

**Effort**: S ≤ 1 day, M a few days, L a week or more.

**Priority**
* 1 = used in most calls
* 2 = common
* 3 = rare, or only for some accounts

---

## 1. Feature table

### A. Call setup

| # | Feature | Official app (where) | Webox status | Needs | Effort | Prio |
|---|---|---|---|---|---|---|
| A1 | 1:1 Webex audio call from a direct space, contact, profile or search | space header call buttons; `PhoneWidgetBody` "Audio call"; "Call %1", "Call on Webex" | **done**: `cloud/locus/session.rs::dial` (`POST {locus}/loci/call`), entry points `ui/conversation.rs:97`, `contacts_page.rs`, `profile.rs`, `global_search.rs` | – | – | 1 |
| A2 | 1:1 **video** call (Video button) | "Video call" (`PhoneWidgetBody`), "Answer calls with my video on" | **missing**: the Video button calls the same audio-only `start_call` (`ui/conversation.rs:90-97`) | media, UI | L | 1 |
| A3 | **Group / space call** ("Meet" in a group space) | space header; dll `sync conversation locus_url`, `join request with loci/participant`, `use loci/call for one2one scheduled call` | **missing**: group spaces show a disabled "Treffen" pill (`ui/conversation.rs:102-106`) | locus, meet, media | L | 1 |
| A4 | Dial by e-mail or contact name (dial pad / search to dial) | `SearchBar` "Search to dial", `Keypad`, "Dialpad" | **done**: `ui/calls_page/mod.rs::dial_text` | – | – | 1 |
| A5 | Dial a **phone number / extension (PSTN)** | "Dial", "Call as {{0}}", "Select a line to call from", "Outgoing Caller ID" | **missing**: `locus::call_allowed` rejects it; `calls_page::no_phone_calling` toast | WxC | L | 2 |
| A6 | Dial a SIP/video address (e.g. `room@site.webex.com`) | "Video address", "SIP you entered is not valid"; "Dialing SIP-URI numbers is not supported." refers to WxC lines | **partial**: a bare `user@domain` passes `call_allowed` and goes to `loci/call`, untested against SIP endpoints; a `sip:` prefix is rejected (`locus/mod.rs:245`) | locus | S | 3 |
| A7 | Call back from call history | `CallHistoryView`, "Call history menu item", "Callback using {{0}}" | **partial**: e-mail only (`calls_page/mod.rs::call_back`); numbers get the upgrade toast | locus (WxC for numbers) | S | 2 |
| A8 | In-app **meeting join** (link, number, calendar row, space meeting) | `MeetingInterstitialPage/*` (pre-join A/V preview), `MeetingConnect`, "Join a meeting" | **partial**: opens the browser or official app (`meetings_page.rs::join_meeting_link`) | meet, locus, media | L | 2 |
| A9 | Start Personal Room meeting | "Start a meeting", `SettingsMeetingGeneralPage` | **partial**: opens the PMR link in the browser (`ui/meetcall.rs::start_personal_meeting`) | meet | M (after A8) | 3 |
| A10 | Choose how to call: Webex call vs. phone call vs. "Call with connected Webex device" / deskphone | "Call %1 with connected Webex device", "Call with Deskphone", "Calling using {{0}}" | **missing** | WxC / device pairing | L | 3 |
| A11 | Line selection for outgoing calls (multi-line) | `PhoneWidgetHeader` "Current line", `LineDelegate`, "Select a line" | **missing** | WxC | M | 3 |

### B. Incoming calls

| # | Feature | Official app (where) | Webox status | Needs | Effort | Prio |
|---|---|---|---|---|---|---|
| B1 | Incoming call toast (caller, avatar) | `Calling/IncomingCallToastView.qml` (`UTIncomingCallToast`), `CallDelegate` "Incoming call" | **done**: `ui/call_toast.rs` (Windows window, Wayland notification), `call_note_macos.rs`, overlay fallback in `ui/call/mod.rs` | – | – | 1 |
| B2 | Accept (audio) | "Answer", "Answer call" | **done**: `CallReq::Answer` → `POST {locusUrl}/participant` | – | – | 1 |
| B3 | Accept **with video** | "Answer calls with my video on" (setting), video answer button | **missing**: the setting `answer_with_video` is stored but unused | media | S once A2 exists | 1 |
| B4 | Decline | "Decline", "Decline Call" | **done**: `PUT {locusUrl}/participant/decline` | – | – | 1 |
| B5 | **Decline and reply with a text** (quick replies) | "Decline call and reply with a text", "Quick Replies ({count})" | **partial**: declines, then opens the chat; nothing is sent (`call_win/call.rs:76`) | UI | S | 2 |
| B6 | Silence the ringer only / ignore the call | `CallDelegate` "Silent call", "Ignore call", "Notification sound, Off" | **partial**: the toast has a mute-ring button; there is no "Ignore" that hides the toast while the call keeps ringing on other devices | UI | S | 2 |
| B7 | Ringtone choice, volume, ring device | `SettingsNotificationsCallsPage` "Direct calls ringtone", flag `desktop-ringer-volume-setting-enabled-ga` | **done**: `ui/call/mod.rs::ring_sound`, `settings/sound_ui.rs` | – | – | 1 |
| B8 | Answered / declined elsewhere | "Answered elsewhere" | **done**: `session.rs::evaluate` (`elsewhere`, `ends_quietly`) | – | – | 1 |
| B9 | **DND / quiet hours suppress call ringing** | presence DND ("Do not disturb until {{0}}"); `SettingsNotificationsCallsPage` `isDirectCallsMuteInCallOption`, "Turn off while in a meeting or call" | **missing**: `ring_sound` and the toast ignore DND and quiet hours | UI | S | 2 |
| B10 | **Call waiting**: second incoming call while in a call (end and accept, hold and answer, decline) | "Hold and Answer", "End Call and Accept", "Answer incoming call and end current call?", "Select a call to end, before answering this call", "Leave meeting && Answer", "Mute meeting && Answer" | **missing**: one `calls.cur` slot; a new incoming locus is ignored while busy (the caller hears ringing until timeout) | locus, UI | M (L with hold) | 2 |
| B11 | Missed-call OS notification + missed badge on the Calls rail/tab | "Missed Call", "Missed calls", "Calls tab, unread voice messages" | **missing**: the call card says "Verpasster Anruf"; there is no badge and no OS toast after the window closes | UI | S | 2 |
| B12 | Auto-answer / answer when the headset is put on / answer on lock screen | "Auto-answer call", "Answer incoming calls when you put the headset on.", "Answer Calls on Lock Screen" | **missing** | UI (auto-answer), accessories | S–M | 3 |
| B13 | Block list applied to incoming calls | `Settings/Calling/SettingsCallingBlockListPage.qml`, `AddBlockNumberDialog` (phone numbers) | **partial**: `block_list` is stored locally (`settings/pages_b.rs:185`) and never consulted | UI (local), WxC for numbers | S | 3 |
| B14 | Group call pickup notifications ("Calls Available For Pickup") | `SettingsNotificationsCallsPage` call pickup section, `WatchlistView` "Pick up %1 %2" | **missing** | WxC | M | 3 |

### C. In-call controls

| # | Feature | Official app (where) | Webox status | Needs | Effort | Prio |
|---|---|---|---|---|---|---|
| C1 | Mute / unmute (local) | "Mute", "Mute or unmute yourself", `PhoneWidgetEnumsWrapper.WidgetCallButtonControlType.Mute` | **done** locally: silence is sent (`media.rs:279`) | – | – | 1 |
| C2 | **Mute state signalled to Locus** (far side sees the muted icon) | dll `call_mute`; webex-js-sdk sends a LocalMute `/media` request | **missing**: `audioMuted` is only sent inside ROAP messages (`locus/mod.rs:322`) | locus | S | 1 |
| C3 | Remote mute by host/server, "unmute request" | "Unmute request sent" (`TopButtonsInPip`), "Moderated unmute mode" (`TitleBarHeader`), event `participant_audio_muted` | **missing** | locus, UI | S | 2 |
| C4 | **Camera on/off** (send video) | "Start video"/"Stop video" (`MeetingInterstitialPage/CallControlBarView`), `CallControlVideoManager` | **missing**: no video track in calls (`media/camera/mod.rs`: "Webex calls are audio only") | media | L | 1 |
| C5 | **Receive and render remote video** | `MeetingViewLayout/VideoPip`, `SingleVideoItem`, `GridVideoPane` | **missing**: video RTP is counted and dropped (`media.rs:234-237`) | media | L | 1 |
| C6 | Switch mic / speaker mid-call | `SwitchAudioView/*`, "Change microphone", "Change speaker" | **done**: `ui/call/devices.rs::device_menu` → `CallReq::Devices` | – | – | 1 |
| C7 | Switch camera mid-call | "Change device", "Camera, {{0}}, selected" | **missing** (needs C4) | media | S | 2 |
| C8 | Self view: show, hide, move, mirror, floating self view | "Hide self view", "Mirror my video view", "Self-view location…", `InnerFloatLocalContainer`, `CropLocalVideoFTE` | **missing** | UI (after C4) | S | 2 |
| C9 | **Hold / resume** a Webex (locus) call | dll `[holdCall] [resumeCall]`, flag `desktop-locus-hold-resume-enabled`, `CompoundDeviceWithLocusShare::hold`; "Hold", "On hold", "Hold/Resume Call" | **missing**: the shortcut `HoldResumeCall` is a stub (`shortcuts/table.rs:113`) | locus, media | M | 2 |
| C10 | Transfer: blind ("Transfer now") and consult ("Consult first", "Complete transfer", "Cancel transfer") | `PhoneWidget/CallDelegate.qml` (`TransferNow`), `MeetingViewLayout/TransferPopover.qml`, dll `call_transfer_type_consult/now`, `[directTransferCall]` | **missing**: shortcut stub `TransferCall` (`shortcuts/table.rs:116`) | WxC | L | 3 |
| C11 | Merge calls / conference ("Merge calls", "Select a call to merge with") | `CallAssociationType.Merge`, dll `[mergeCall]` | **missing** | WxC | L | 3 |
| C12 | **Add people to a 1:1 call** → "Invite and meet" / "Move to Meeting" (escalation) | `EscalationInviteAndMeet/EscalationInviteAndMeetView.qml` (invitee list, `meetNow()`), "Make this a meeting by inviting others or by clicking Meet now.", dll `escalateToMeeting`, `escalationMeetNowAction`, `escalateCallToMeetingNotificationArrived`, `AddPersonAction` | **missing** | locus, meet, media | L | 2 |
| C13 | Invite / add participant to a group call or meeting | `ParticipantList/InvitePeopleAndDevicePage`, `InvitePhonesPage`; dll `call_addParticipant`, `call_addGuest`, `call_addPSTNGuest`, `call_addSipDeviceGuest` | **missing** | locus | M (after A3) | 2 |
| C14 | **Move call** to another device / pull the call here | "Move Call Here", "Move call to device", "Call pull, move an active call here", "Always move call if supported"; dll `[moveMedia]`, `call_move`, `call_pull` | **missing**: the join body only has the literal `moveMediaToResource:false` | locus | M | 3 |
| C15 | Park / retrieve | "Park", "Call parked at {{0}}", "Call park will expire soon", dll `[parkCall] [resumeCallPark]` | **missing** | WxC | M | 3 |
| C16 | Call pickup (group / directed) | "Call Pickup", "Directed Pickup", "Enter pickup number", dll `call_pickup` | **missing** | WxC | M | 3 |
| C17 | DTMF keypad | `PhoneWidget/Keypad.qml` (passcode hide/show), dll `call_sendDtmf`, `handleDTMFPaste` | **done**: `ui/meetcall.rs::call_keypad_ui` → `POST {locusUrl}/sendDtmf`. Paste of digit strings is not checked | – | – | 2 |
| C18 | Recording (meeting cloud recording; WxC "Call recording") | `MeetingContent/RecordingControls`, `Streaming/RecordControl`, `WxccRecordingFloatingWindow`, dll `call_recordMeeting`, `call_pauseRecording` | **missing**. Unsure whether the official app offers Record in a plain Webex 1:1 call; the strings tie it to meetings and to Webex Calling | locus (meetings) / WxC | M | 3 |
| C19 | Raise / lower hand | "Raise hand", "Lower hand", `CallControlRaiseHandReactionManager`, dll `call_toggleAllowRaiseHand` | **missing**. Meetings and group calls; it probably does not apply to 1:1 calls | locus | S | 3 |
| C20 | **Reactions** (send and show, skin tone) | `MeetingViewLayout/Reaction.qml`, `ReactionListOverlay.qml`, `Calling/QMLReactionSkinFTEWidget.qml`, "Send reaction", "Reaction limit reached", dll `call_toggleMeetingReaction`, `locus data channel url changed` | **missing**: the shortcuts are stubs | locus (LLM data channel) | M | 2 |
| C21 | Closed captions / transcription | `PhoneWidget/ClosedCaptions.qml`, `MeetingViewLayout/ClosedCaptionBanner.qml`, `CaptionsHighLights/*`, dll `call_toggleTranscription`; "Closed caption feature requires this call to be moved to a meeting experience" (for WxC calls) | **missing** | locus + data channel; licence | L | 3 |
| C22 | Video layout: grid / stack / side by side / focus, videos per page, zoom | `MeetingViewLayout/VideoLayoutMenu.qml`, `CallTitleBar` "Layout", "Increase or decrease video size", `SettingsMeetingLayoutPage` | **missing**: only the `grid_videos` setting is stored | media (multistream), UI | L | 2 |
| C23 | Calling dock / floating mini window / keep in front | `PhoneWidget/PhoneWidgetView.qml`, `Settings/Calling/SettingsCallingDockPage.qml`, "Automatically show my {{APP_NAME}} call or meeting in a floating mini window when other apps are in front." | **done**: `ui/call_win/dock.rs`. Bug: the settings page writes `dock_stay_on_top` / `calling_dock` / `multitasking`, but the dock reads `dock_keep_in_front` / `dock_auto_covered` | UI | S (key fix) | 2 |
| C24 | Separate call window, maximise, full screen | `CallWindowDialog/CallWindowView.qml`, `WindowButtons.qml`, `MeetingViewButtons` "Full screen" / "Exit full screen" | **partial**: own borderless window with maximise (`call_win/call.rs`); no true full screen | UI | S | 3 |
| C25 | Call timer | `CallDelegate` `durationTime` | **done**: `ui/call/mod.rs::state_text` | – | – | 1 |
| C26 | **Call statistics / network quality** | "Show Call Statistics", "Statistics", "Packet loss", "Jitter", "Latency", "Codec", "Resolution", "Network & CPU is good", "Poor network connection…", "Call is recovering..." | **partial**: debug-log counters every 10 s (`STATS_EVERY`), no UI | media (stats), UI | S–M | 2 |
| C27 | Noise removal on my mic | "Noise Removal", `SettingsSmartAudioPage` | **done**: `media/denoise.rs` (RNNoise / DeepFilterNet 3), `media/mic.rs` AEC3/AGC, `CallReq::SmartMic` | – | – | 1 |
| C28 | Noise removal for the **other caller** (receive side) | "Remove background noise from the other caller?", "Noise removal for the other caller is on/off", "Noise detected from the other caller…" | **missing** | media | S | 3 |
| C29 | Music mode / "Optimize for all voices" | "Music mode", "Optimize for all voices", "Optimize for my voice" | **partial**: Smart Audio modes exist; no "music mode" (denoise + AGC off) | media | S | 3 |
| C30 | Background blur / virtual background / touch up | `Settings/SettingsVideoBackgroundPage.qml`, `MeetingInterstitialPage/VirtualBackgroundSettingView.qml`, "Blur", "Change virtual background" | **missing**: the settings page is hidden (`settings/nav.rs:9`) | media (segmentation) | L | 3 |
| C31 | End call / leave | "End call", `WidgetCallButtonControlType.EndCall` | **done**: `PUT …/participant/{id}/leave` | – | – | 1 |
| C32 | End meeting for all | `EndMeeting/*`, "Leave or end the meeting or call" | **missing** (meetings and group calls only) | locus | S | 3 |
| C33 | In-call chat panel | `MeetingChat/*`, "Chat panel"; `CallControlSidePaneManager` | **missing** | UI (reuse the conversation view) | M | 2 |
| C34 | **Participants panel** (roster, states, mute others, remove, admit from lobby) | `ParticipantList/*`, `PhoneWidget/ParticipantsPopup.qml` ("Remove from call", "%1 people in the conference call"), `MuteOptionsMenu`, `LobbyParticipantList/*`, dll `call_removeParticipant`, `call_admitParticipant` | **missing**: `parse_locus` keeps only the others' name, e-mail and state | locus, UI | M | 2 |
| C35 | Screen share **send** | `Sharing/ShareDialog/*`, `LocalShareControlBar/*` | **done** (PR #20): `ui/share_picker/`, `media/share/`, `cloud/locus/share.rs` floor GRANTED/RELEASED | – | – | 1 |
| C36 | Screen share **receive** | `Sharing/InMeeting/RemoteShareView.qml`, `ContentShare.qml`, `FloatingVideoContainer` (swap content/video) | **in progress** (another agent) | media | L | 1 |
| C37 | Share computer audio | "Share computer audio", `ShareControlBarMoreMenu`, `ShareAudioInControlBar` | **missing**: the picker checkbox sets `Choice.audio`, which is not carried in `ShareReq`; `audio/slides` is offered inactive | media | M | 3 |
| C38 | Sharing control bar, "you're sharing" border, pause/resume share, optimize for text/video | `LocalShareControlBar*`, `SharePortionDialogBorder`, "Pause sharing (Alt+Shift+S)", `shareOptimizeForText/Video` | **missing** (no border, no floating bar, no pause) | UI, media | M | 2 |
| C39 | Remote control (give/request), annotation | `RemoteControl/*`, `Annotation/*`, dll `call_toggleRdcControl` | **missing** | locus, media, input injection | L | 3 |
| C40 | Active speaker indication / speaking bubble | `MeetingViewLayout/ActiveSpeakerWave.qml`, `CallSpeakingBubble.qml` | **missing** | media (audio level), UI | S (1:1) / M (group) | 2 |
| C41 | Step away (mute + video off + "Stepped away" status) | `TopButtonsInPip` "Step away from meeting", `SettingsMeetingGeneralPage` | **missing** | locus, UI | S | 3 |
| C42 | Far-end camera control | `FECC/*`, `MeetingViewButtons` "Far end camera control" | **missing** | locus/media | M | 3 |
| C43 | **Media recovery** (ICE restart / rejoin, "Call is recovering...", "Call lost connection to audio or video.") | dll `call_reconnect`, `[reconnectCall]`, `onMediaDisconnected`, `desktop-locus-resilience-enabled-dev` | **partial**: `media_stall` warns and keeps the call; no ICE restart or rejoin; the `Reconnect` sound is unused | locus, media | M | 1 |
| C44 | TCP / TLS-443 media fallback | `call-flow-spec.md` §2 (`ice-options:cisco+tls+fqdn`, TCP 5004, xTLS 443) | **partial**: UDP only (`sdp::udp_only`) | media | L | 2 |
| C45 | In-call keyboard shortcuts (mute, video, end, hold, share, raise hand) | `SettingsKeyboardShortcutsPage`, "Hold/Resume Call", dll `getButtonShortcutType` | **partial**: AnswerCall, DeclineCall, EndCall and ToggleAudioMute work (`shortcuts/commands.rs`); hold, transfer and reactions are stubs | UI | S per action | 2 |
| C46 | Auto-hide call controls | "Automatically hide controls", flag `desktop-auto-hide-call-control-enabled` | **done**: `call_autohide_controls` in `call_win/call.rs` | – | – | 2 |
| C47 | Headset button mute / hook sync | "Dock headset to end an active call.", `desktop-sync-system-mute-unmute-for-cucm-calling`, `Accessories/*` | **missing** | HID | M | 3 |

### D. Call types

| # | Feature | Official app (where) | Webox status | Needs | Effort | Prio |
|---|---|---|---|---|---|---|
| D1 | Webex 1:1 call (locus `CALL`) | `call-flow-spec.md` §1 | **done** (audio, plus share send) | – | – | 1 |
| D2 | Space meeting (group space locus) | see A3 | **missing** | locus, media | L | 1 |
| D3 | Scheduled / PMR meeting in-app | see A8, `MeetingInterstitialPage` | **partial** (browser hand-off) | meet, locus, media | L | 2 |
| D4 | Escalate 1:1 to a meeting | see C12 | **missing** | locus, meet | L | 2 |
| D5 | Join a meeting while in a call ("Hold Call && Join", "End Call && Join"), flag `desktop-join-meeting-in-call-enabled` | strings | **missing** (needs B10 multi-call state) | locus | M | 3 |
| D6 | **Webex Calling** (PSTN / extension) softphone calls | `PhoneWidget/*` lines, "Phone Service", "Phone services are connecting...", `BroadWorksCalling.dll`, `spark-windows-bwc.dll` | **missing** | WxC | L | 2 (licence) |
| D7 | UCM / deskphone control (DPC) | "Call with Deskphone", `CallControlUCMAndBWCManager`, `JabberCalling.dll` | **missing**: out of scope for Webox | UCM | L | 3 |
| D8 | Meeting audio by phone ("Call me", "Call in") | `SwitchAudioView/CallMeItem.qml`, `NativeCallInView/*` | **missing** | meet | M | 3 |

### E. History and voicemail

| # | Feature | Official app (where) | Webox status | Needs | Effort | Prio |
|---|---|---|---|---|---|---|
| E1 | Call history list (all / missed), details, delete | `Calling/CallHistoryView.qml`, "Call history", "Missed calls" | **done**: janus `GET …/history/userSessions`, `markAsDeleted`, `ui/calls_page/mod.rs` | – | – | 1 |
| E2 | Missed-calls / unread-voicemail badge | "Call history tab, unread voice messages", flag `desktop-voicemail-mwi-count-enabled-ga` | **missing** (see B11) | UI (missed), WxC (MWI) | S | 2 |
| E3 | Call back from history (audio / video / number) | history row actions | **partial**: e-mail only, audio only | locus / WxC | S | 2 |
| E4 | Voicemail list, play, mark read, delete, transcript, call back, forward, reply, record a voice message | `Voicemail/*` (`VoicemailPagedListDelegate` "Play voicemail", "More voicemail actions"; `VMRecordingView`, `VMRCaptureArea`), "Forward voice message", "Reply voice message", `VoicemailService.dll`, flag `desktop-broadworks-voicemail-transcription-enabled-v2` | **missing**: UI only on `--sample` data (`calls_page/voicemail.rs`); the real list is empty | WxC | L | 2 (licence) |
| E5 | Group voicemail | "New group voicemail", `desktop-broadworks-group-vm-*` | **missing** | WxC | M | 3 |
| E6 | Post-call summary / AI call notes | `NotificationCallSummaryDialog.qml`, "Call notes", `desktop-broadworks-post-call-summary-enabled-ga` | **missing** | AI licence | L | 3 |
| E7 | Call summary rows in the space ("You had a call") | `locusSessionSummary` activity | **done**: `cloud/activities/fold.rs:79` | – | – | 2 |

### F. Settings

| # | Feature | Official app (where) | Webox status | Needs | Effort | Prio |
|---|---|---|---|---|---|---|
| F1 | Audio devices, test, level meter | `Settings/SettingsAudioPage.qml`, `SettingsAudioLevelMeter.qml` | **done**: `ui/call/devices.rs::device_pickers` | – | – | 1 |
| F2 | Video settings (camera, resolution, mirror, touch up) | `Settings/SettingsVideoPage.qml` | **partial**: camera choice and preview only, no effect on calls (`settings/video_page.rs`) | media (after C4) | S | 2 |
| F3 | Background (blur / image) | `SettingsVideoBackgroundPage.qml` | **missing** (page hidden) | media | L | 3 |
| F4 | Call sounds: ringtone, pickup tone | `Settings/Notifications/SettingsNotificationsCallsPage.qml` | **done** (ringtone); pickup tone stored only | – / WxC | – | 1 |
| F5 | "Answer calls with my video on" / "Answer calls with speaker on" | `SettingsCallingGeneralPage` (`videoOption`) | **partial**: stored, unused (`settings/pages_b.rs:160`) | media | S | 2 |
| F6 | Calling dock settings (show dock, keep in front, incoming call behaviour) | `Settings/Calling/SettingsCallingDockPage.qml` | **partial**: works, but with key mismatches (see C23) | UI | S | 2 |
| F7 | Call forwarding (always / to number, reminder banner) | `SettingsCallingGeneralPage` `forwardOptions`, `AddForwardNumberDialog`, "Forwarding calls to {{0}}" | **partial**: a local radio with no API (`pages_b.rs:168`) | WxC | M | 2 (licence) |
| F8 | Single number reach / alternate numbers / Office Anywhere | `SnrLocationDialog.qml`, `alternateNumber`, `setSnrEnabled` | **missing** | WxC | M | 3 |
| F9 | Call pickup, hunt group, call queues, executive assistant | `SettingsCallingGeneralPage` (`pickupSectionModel`, `queuesSectionModel`, `assistantSectionModel`), "Hunt group enabled" | **missing** | WxC | M | 3 |
| F10 | Calling block list (numbers) | `SettingsCallingBlockListPage.qml` | **partial**: local list, not applied (see B13) | WxC / UI | S | 3 |
| F11 | Hold reminder, "bring call into view on hold", caller intent, call review | `SettingsCallingGeneralPage` `holdOption`, `intentOption`, `reviewOption` | **partial**: `bring_hold_into_view` is stored, nothing uses it | locus/WxC | S (after C9) | 3 |
| F12 | Preferred answering device / call pull settings | `preferredDevices`, `pullOptions` | **missing** | WxC | M | 3 |
| F13 | E911 location | `E911Disclaimer/*`, "Emergency 911 Service", `desktop-calling-e911-location-support-enabled` | **missing** (US/CA only) | WxC | M | 3 |
| F14 | Meetings: join options (mute on join, video on join), grid videos per page, lobby | `Settings/Meeting/*` | **partial**: stored, unused (no in-app meetings) | meet | S (after A8) | 3 |
| F15 | "Additional call settings" / user portal link | "Access User Portal", "Advanced Call Settings" | **done**: opens settings.webex.com | – | – | 3 |
| F16 | Auto-decline while in a meeting or call (send to voicemail) | "Automatically decline calls while in a meeting or call, and send call to voicemail or another number." | **missing** | WxC | S | 3 |

**Counts (rows A1–F16, 103 rows):**

| status | rows |
|---|---|
| done | 22 |
| partial | 22 |
| missing | 58 |
| in progress | 1 (share receive) |

Some rows overlap on purpose so each section is complete. These are D2/A3, D4/C12, E2/B11 and F10/B13.

---

## 2. Implementation notes for missing and partial features

The notes are grouped by mechanism, because many features share one. Endpoint shapes marked *(sdk)* come from the public webex-js-sdk `plugin-meetings` / `@webex/calling` behaviour and have **not been seen on the wire for this account**. Confirm them against a live trace before implementing, as `call-flow-spec.md` did for the 1:1 flow.

### 2.1 Locus REST additions (participant and locus controls)

* **C2 mute to Locus.**
  * Send a `PUT {self.url}/media` with `localMedias:[{localSdp: JSON{audioMuted, videoMuted}, mediaId}]`. It carries no `roapMessage`; webex-js-sdk calls this a LocalMute media request *(sdk)*.
  * Webox already builds that body in `locus/mod.rs::roap_local_sdp`. Add a variant without the ROAP part, and call it from `CallReq::Mute` in `session.rs:10` once `mediaId` is known.
  * dll evidence: `call_mute`, `participant_audio_muted` events (`protocol_coverage.md` §1).
* **C3 remote mute.**
  * Read `self.controls.audio.muted` and `self.status.audioStatus` from the locus delta. These already arrive through `apply_event`.
  * Map them to `CallEvent::RemoteMuted` and show a banner. The "Unmute request sent" flow answers with the same `/media` request.
* **C34 participants panel, C13 add, C32 end for all, C19 raise hand.** All of these are *(sdk)*:
  * mute other: `PATCH {locusUrl}/participant/{pid}/controls {"audio":{"muted":true}}`
  * raise or lower hand: `PATCH …/participant/{pid}/controls {"hand":{"raised":bool}}`
  * remove: `PUT {locusUrl}/participant/{pid}/leave {"reason":…}`
  * admit from lobby: `PATCH {locusUrl}/controls {"admit":{"participantIds":[…]}}`
  * invite: `PUT {locusUrl}/participant {"invitees":[{"address":email}],"alertIfActive":true}`
  * end for all: `POST {locusUrl}/end`

  dll ids `call_removeParticipant`, `call_admitParticipant`, `call_addParticipant`, `call_addGuest`, `call_toggleAllowRaiseHand` confirm that the features exist.

  Webox changes:
  * a new `cloud/locus/controls.rs` holding these requests;
  * `parse_locus` (`locus/mod.rs`) keeps the full participant list (id, url, state, `controls.audio.muted`, `controls.hand.raised`, `isHost`/roles);
  * a new `ui/call_win/roster.rs` side panel.
* **C14 move call / call pull.**
  * The official client re-joins the locus from the target device and drops the old device (dll `[moveMedia]`, `Move_Media: Locus joined and Moved`, `Local Device Alreay Joinned`, `rejoinLocusWithDevice`).
  * In the join body these are the `moveMediaToResource` / `usingResource` fields that Webox already sends as literals *(sdk: `moveToResource:true, resourceId`)*. "Move call here" for a call active on my other device is a join on `{locusUrl}/participant` with this device, followed by Locus removing the other device.
  * Webox changes: allow `answer()` on a locus where self is already `JOINED` on another device (today `evaluate` treats that as "answered elsewhere"), and add a "Hierher holen" button on the history/active-call row.
* **C9 hold / resume (Webex locus call).**
  * The feature exists behind `desktop-locus-hold-resume-enabled` (dll), next to `CompoundDeviceWithLocusShare::hold`.
  * **The wire mechanism was not found offline.** It is most likely either a ROAP re-OFFER with `a=inactive`/`sendonly` (standard SDP hold) or a locus participant control.
  * Implement only after a trace of the official client holding a Webex call. Until then, a local hold (mute the mic, pause playback, play hold music) is possible but invisible to the far side.
* **C41 step away.** Mute (C2) + video off + presence "stepped away". It is meetings-only, so it waits for A8.

### 2.2 Group / space calls and meetings (A3, A8, A9, D2–D5, C12)

* **Space call (A3).** The dll strings show the official flow:
  1. Sync the conversation's `locusUrl` (`sync conversation locus_url`, `Found conversation for locus url`).
  2. Then either `join request with loci/participant` when a locus exists, or `join request with loci/call` with the space as the invitee when none exists.

  *(sdk)*: `POST {locus}/loci/call` with `invitee.address` = the conversation URL / space SIP address. Newer orgs resolve the space through `wbxappapi` meetingInfo (dll: `fetch from wbxappapi`).

  Webox changes:
  * keep `locusUrl` on `Space` (`cloud/lists.rs`);
  * a `CallReq::JoinSpace(conv_id)` in `session.rs` that reuses `answer()`'s join body;
  * enable the "Treffen" pill in `ui/conversation.rs`;
  * `idle_event_needs_read` must stop ignoring non-CALL loci that belong to a known space, so "meeting in progress" can be shown.

  Audio works with the existing single audio m-line, because the server mixes the audio (`serverComposed`, `preferTranscoding`; `call-flow-spec.md` §4).
* **In-app meeting join (A8).**
  1. Resolve link, number or SIP to `meetingInfo` (wbxappapi). This gives `locusUrl` or `sipUrl`, plus password/captcha/registration needs (dll: `Need clean cached meetingInfo…`).
  2. Join as in A3.
  3. Handle the lobby (`self.state == IDLE` with `lobby` / `waitingForOthers`; `LobbyParticipantList/*`).

  Add the pre-join screen (`MeetingInterstitialPage`: device choice, mute/video on join).

  Webox: a new `cloud/locus/join.rs`, the `meetings_page.rs` join button, and the `settings` `join_*` keys become effective (F14).
* **Escalate 1:1 to a meeting (C12/D4).**
  * Evidence: `EscalationInviteAndMeetView` (invitees + "Meet now"), dll `escalateToMeeting`, `escalationMeetNowAction`, `escalateCallToMeetingNotificationArrived`, `[escalateCall] [deescalateCall]`.
  * The other side receives a notification and joins the new meeting.
  * **The request was not decoded.** A plausible path is to create an instant meeting (Webex `POST /v1/meetings` with `adhoc`/`scheduledType` instant, *unverified*), invite the 1:1 peer + new invitees, and move both, but this must be traced.
  * Do this after A8, because it lands in the meeting UI.
* **Call waiting (B10).**
  * Today `session.rs` has a single `calls.cur`. It needs a second slot, `pending_incoming`, so a ringing call can be shown while connected.
  * Actions:
    * "End & accept": leave cur, then answer.
    * "Decline": the existing decline.
    * "Hold & answer": needs C9.
  * The ring uses the call-waiting beep, not the full ringtone.

### 2.3 Media (video, stats, recovery)

* **C5 receive video / C4 send video / A2 / B3 / C7 / C8.**
  * In Webox the main video m-line is already offered (inactive) since PR #20 (`Media::add_share`).
  * For a 1:1 call the server transcodes (`preferTranscoding:true`), so a single H.264 `video/main` `sendrecv` line is enough. Neither multistream nor simulcast is required (`call-flow-spec.md` §4, §6).
  * Receiving reuses the decoder and texture path the share-receive agent is building now (`rx_video`), so **start after share receive is merged**.
  * Sending reuses the H.264 encoder from `media/share/` with the camera frames from `media/camera/`. It needs a ROAP re-OFFER that changes `video/main` to `sendrecv` (the same `<update_media>: roap flow` as share).
  * `videoMuted` in `localSdp` must follow the camera state (C2's request).
* **C22 layouts / C40 active speaker.**
  * For group calls with a transcoded single stream, the server composes the layout, and the client can ask for one with `PUT …/participant/{pid}/controls {"layout":{"type":…}}` *(sdk; dll `call_setMeetingLayout`)*.
  * True client-side grid/stage needs Cisco multistream (CSI subscriptions, `a=content:main` multiple streams) — **L, defer**.
  * In 1:1, active speaker = the remote audio level (RTP `ssrc-audio-level` extmap), which is easy.
* **C26 stats.**
  * webrtc-rs exposes `get_stats()` with inbound-rtp `packetsLost`/`jitter` and candidate-pair `currentRoundTripTime`.
  * Add a `Media::stats()` snapshot (replacing the debug log), a small panel in `call_win`, and a "poor network" banner when loss is above 5 % or RTT above 400 ms.
* **C43 recovery.**
  * The official client keeps the call through `IceFailure`/`Lost`/`Reconnected` (dll `[reconnectCall]`, `call_reconnect`).
  * Implement an ICE restart: a re-OFFER with new ufrag on the same `mediaId`. If that fails after N seconds, rejoin (`POST {locusUrl}/participant` with a fresh offer) while self is still in the locus.
  * Play the existing `Reconnect` sound and show "Verbindung wird wiederhergestellt…".
* **C44 TCP/TLS fallback.** webrtc-rs has no ICE-TCP/TLS-443. This needs a custom transport (`ice_socket.rs`), so it is L and only worth it on UDP-blocked networks.
* **C28 / C29 audio.**
  * C28: run the existing `denoise.rs` on the decoded far-end audio before playback.
  * C29: music mode is AEC on with NS and AGC off, as a new `SmartMic` variant.
* **C37 share audio.** Capture system loopback (WASAPI loopback / PipeWire monitor / ScreenCaptureKit audio), Opus-encode it, and send it on `audio/slides` (set the m-line to `sendonly` in the share re-OFFER). Carry `Choice.audio` through `ShareReq`.
* **C30 virtual background.** This needs person segmentation (e.g. MediaPipe selfie model through ONNX). It is L, comes after C4, and is low priority. Mind the memory and CPU budget (Webox must use less than official Webex).

### 2.4 Data channel features (C20 reactions, C21 captions)

* Locus exposes `locus.info.datachannelUrl` (dll: `locus data channel url changed`).
* The client registers *(sdk: `POST {datachannelUrl}` → `{webSocketUrl, binding}`)* and opens a second websocket (LLM).
* **Reactions:** publish `{eventType:"relay.event", relayType:"client.reaction", reaction:{type, codepoints, tone}, sender:{participantId}}` *(sdk)* and receive the same event.
* **Captions:** `PATCH {locusUrl}/controls {"transcribe":{"caption":true}}` *(sdk; dll `call_toggleTranscription`)*. The caption events then come over the same LLM socket.
* Captions need the org's AI/assistant entitlement. For WxC calls the official app requires "move to meeting" (string quoted in C21).
* Webox: a new `cloud/llm.rs` (reusing the Mercury websocket code), a reactions overlay in `call_win`, and a captions strip.
* It is **unknown whether reactions are offered in plain 1:1 Webex calls**. The managers are shared with meetings, so test this in the official client first.

### 2.5 UI-only items

* **B5 decline with text.**
  * Add a quick-reply menu with three fixed strings plus custom text to the toast (`call_toast.rs`) and the call card.
  * On choice, decline, then send the text through the existing message send path (`Req::Send` to the 1:1 space).
* **B6 ignore.** Hide the toast and stop the local ring without `decline`, so other devices keep ringing. The locus stays NOTIFIED until the caller gives up.
* **B9 DND.**
  * In `ui/call/mod.rs::ring_sound` and the toast, skip ringing and the toast when my presence is DND or quiet hours are active.
  * Still record the call as missed (B11). This matches the official "Do not disturb" behaviour of muting notifications. Whether the official app hides the call toast entirely under DND, or only silences it, is **not confirmed**. Silence + a compact toast is the safe default.
* **B11 missed badge.** Count janus rows with `missed && !seen` since the last Calls-tab visit, show a badge on the rail, and send an OS notification on `Ended("missed")`.
* **B12 auto-answer.** Answer after N seconds (a settings key) when there is no active call; the headset/lock-screen variants are left out.
* **B13 block list.** Before alerting, compare the caller's e-mail to `block_list` and decline silently.
* **C23 / F6 dock keys.** Unify the keys `dock_stay_on_top` → `dock_keep_in_front` and `multitasking` → `dock_auto_covered`, with a one-time migration in settings load. Also revive `toggle_calling_dock` for the Ctrl+Shift+X shortcut (`shortcuts/commands.rs:39`).
* **C24 full screen.** Use `ViewportCommand::Fullscreen` on the call viewport.
* **C33 chat panel.** Embed the 1:1 space's conversation view in a right side panel of the call window.
* **C38 share bar.**
  * Add a borderless always-on-top bar on the shared screen (Stop / Pause / annotate placeholder) and a 2 px border window around the shared monitor.
  * Pause = stop feeding frames but keep the floor.
* **C45 shortcuts.** Bind them to the new `CallReq`s as each feature lands.

### 2.6 Webex Calling (licence-gated, see section 3)

* The official desktop app is a **SIP softphone** for Webex Calling / BroadWorks (`BroadWorksCalling.dll`, `bwc.dll`, `spark-windows-bwc.dll`, the `desktop-broadworks-*` toggles, "Phone services are connecting...").
* For Webox, the realistic route is the **WebRTC gateway (Mobius)** that the Webex web client uses *(sdk `@webex/calling`, unverified for this account)*:
  * register: `POST {mobius}/calling/web/device`;
  * place or answer calls with ROAP over the same OFFER/ANSWER/OK machinery Webox already has;
  * call control on the call resource: hold, resume, blind and consult transfer.
* Voicemail and call settings (forwarding, DND, call waiting) go through the user's BroadWorks XSI or the Webex Calling user settings APIs *(sdk `VoicemailClient` / `CallSettings`)*.
* Before any of this, add an entitlement probe: Webex `GET /v1/people/me?callingData=true` (phone numbers / extension present) and the wdm device `callingBehavior`. Hide every WxC row when the probe fails.
* Webox: a new module tree `cloud/wxc/` (`register.rs`, `call.rs`, `voicemail.rs`, `settings.rs`); `calls_page/voicemail.rs` switches from sample data to `wxc::voicemail`; `call_allowed` accepts E.164 / extension numbers when entitled.

---

## 3. Features that need a Webex Calling licence or an admin setting

These do not work on a messaging/meetings-only account. The official app hides them unless the user has a calling line (strings "Phone Service unavailable", "Phone services required", "Calling entitled", toggle `call-entitlements-honored`). **It is not known whether the account used for Webox testing has Webex Calling.** The Webox voicemail page reports "not set up", which is consistent with **no** licence. Run the entitlement probe (2.6) first.

**Paid Webex Calling (or BroadWorks / UCM) line**
* A5 PSTN / extension dialing
* A7 / E3 call back to numbers
* A10 call via deskphone / device
* A11 line selection
* D6 softphone calls
* D7 UCM / DPC
* C10 transfer
* C11 merge
* C15 park
* C16 pickup
* B14 pickup notifications
* E4 / E5 voicemail and group voicemail
* E2 voicemail MWI badge
* F7 call forwarding
* F8 SNR / alternate numbers
* F9 hunt group / queues / executive assistant
* F10 number block list
* F12 preferred device / call pull settings
* F13 E911
* F16 auto-decline to voicemail
* C18 call recording (WxC "Call recording")
* C9 hold of **WxC** calls. Hold of Webex locus calls is a feature toggle, not a licence.

**Admin setting, org policy or add-on**
* C21 captions / transcription (Webex Assistant / AI entitlement; for WxC calls only after moving to a meeting)
* C18 meeting recording (site/org recording rights; "Recording to the cloud is unavailable")
* C39 remote control (org policy)
* E6 AI call summary
* A8 / A9 meetings (a Webex Meetings site is needed for the PMR; "Your account currently does not have the ability to start a meeting.")
* C12 escalation (same site requirement)
* C9 locus hold (`desktop-locus-hold-resume-enabled` toggle)

---

## 4. Batch plan

Each batch has 3–6 features. **Lanes** within a batch touch disjoint files, so up to three agents can run in parallel; within a lane, the work is sequential. Hot files and their owners:
* `cloud/locus/session.rs` / `locus/mod.rs`: the "locus" lane;
* `cloud/media.rs`: the "media" lane, and **only after share receive is merged**;
* `ui/call_win/call.rs`: the "call UI" lane.

### Batch 1: quick wins on existing 1:1 calls (prio 1–2, all S)

| Lane | Features | Files | Acceptance test |
|---|---|---|---|
| locus | C2 mute to Locus, C3 remote-mute banner | `cloud/locus/session.rs`, `locus/mod.rs`, `ui/call/mod.rs` (event only) | Mute in Webox → the official client on the other side shows the muted icon within 2 s; unmuting clears it |
| ring/toast | B9 DND/quiet hours silence calls, B6 Ignore, B5 decline with quick reply, B13 local block list | `ui/call_toast.rs`, `ui/call/mod.rs::ring_sound`, `call_note_macos.rs` | With DND on, an incoming call does not ring; "Ablehnen mit Nachricht → Kann gerade nicht" declines and posts that text in the 1:1 space |
| calls page / settings | B11/E2 missed badge + OS notification, C23/F6 dock key fix + Ctrl+Shift+X | `ui/calls_page/mod.rs`, rail badge in `ui/chrome/`, `ui/settings/pages_b.rs`, `shortcuts/commands.rs` | A missed call shows a "1" badge on Anrufe until the tab is opened; toggling "Im Vordergrund halten" in settings changes the dock behaviour |

### Batch 2: group calls and roster (prio 1–2)

| Lane | Features | Files | Acceptance test |
|---|---|---|---|
| locus (join) | A3/D2 space call (audio), locus `locusUrl` on spaces, "meeting in progress" pill | `cloud/locus/session.rs`, `cloud/lists.rs`, `ui/conversation.rs` | Click "Treffen" in a group space → two official clients that join the same space meeting hear Webox, and Webox hears both |
| locus (controls) | C34 participants panel (list, mute other, remove), C13 invite, C19 raise hand, C32 end for all | new `cloud/locus/controls.rs`, `locus/mod.rs::parse_locus` (this lane owns `mod.rs`), new `ui/call_win/roster.rs` | In a space call the roster lists all joined people with mute state; "Stummschalten" on another participant mutes them in their official client |
| media-free UI | C26 stats panel (webrtc-rs `get_stats`, read-only accessor), C24 full screen, C45 shortcuts for the new actions | new `ui/call_win/stats.rs`, a small read-only `Media::stats()` (coordinate with the share-receive agent), `shortcuts/commands.rs` | "Anrufstatistik" shows packet loss / jitter / RTT updating every 2 s; F11 toggles full screen |

### Batch 3: video (prio 1; start after share receive is merged)

| Lane | Features | Files | Acceptance test |
|---|---|---|---|
| media (owner of `media.rs`) | C5 receive main video, C4 send camera video, ROAP re-OFFER for `video/main` | `cloud/media.rs`, `cloud/sdp.rs`, `media/camera/`, `media/video/` | In a 1:1 Webox↔official call both sides see each other's camera at ≥ 15 fps |
| call UI | A2 video call button, B3 answer with video (+ F5 setting), C7 camera switch, C8 self view (hide/move/mirror) | `ui/conversation.rs`, `ui/call_toast.rs`, `ui/call_win/call.rs`, `ui/call/devices.rs` | The Video button starts a call with the camera on; switching the camera mid-call changes the far side's picture without dropping the call |
| settings | F2 video settings effective (resolution cap, mirror), C40 active-speaker ring (1:1 audio level) | `ui/settings/video_page.rs`, `ui/call_win/call.rs` (small tile overlay; coordinate with the call UI lane) | The 720p cap in settings limits the sent resolution (visible in C26 stats) |

### Batch 4: resilience and meetings (prio 1–2)

| Lane | Features | Files | Acceptance test |
|---|---|---|---|
| locus/media | C43 ICE restart + rejoin, "recovering" UI + Reconnect sound | `cloud/locus/session.rs`, `cloud/media.rs` | Disable Wi-Fi for 10 s mid-call → the call resumes with audio, without the user acting |
| meetings | A8 in-app meeting join via meetingInfo, lobby state, pre-join device screen, F14 join options | new `cloud/locus/join.rs`, `ui/meetings_page.rs`, new `ui/call_win/prejoin.rs` | Joining a scheduled Webex meeting from the Meetings page puts Webex in the meeting (lobby → admitted) with audio |
| side panel | C33 in-call chat panel, C38 sharing bar + border + pause | new `ui/call_win/chat.rs`, new `ui/share_bar.rs`, `cloud/locus/share.rs` (pause only) | While sharing, a floating bar with Stop/Pause is on the shared screen; the chat panel posts into the 1:1 space |

### Batch 5: call control parity on Webex calls (prio 2; needs live traces first)

| Lane | Features | Files | Acceptance test |
|---|---|---|---|
| locus (multi-call) | B10 call waiting (end and accept / decline), C9 locus hold/resume (after the trace), F11 hold reminder | `cloud/locus/session.rs` (second call slot), `ui/call/mod.rs` | During a call a second Webex call shows a waiting toast; "Beenden und annehmen" connects the new caller |
| escalation | C12 invite-and-meet escalation, D5 join meeting while in call | new `cloud/locus/escalate.rs`, new `ui/call_win/invite.rs` | Adding a third person in a 1:1 moves all three into one meeting |
| move | C14 move call here / to my other device, C20 reactions over the LLM data channel | `cloud/locus/join.rs` (move flag), new `cloud/llm.rs`, `ui/call_win/reactions.rs` | A call answered on the phone app can be pulled to Webox with "Hierher holen"; a 👍 reaction sent from Webox appears in the official client |

### Batch 6: Webex Calling (licence-gated, prio 2 for entitled accounts)

Prerequisite: the entitlement probe succeeds on a test account.

| Lane | Features | Files | Acceptance test |
|---|---|---|---|
| wxc core | Mobius registration, A5/D6 PSTN/extension dialing and receiving, C9 WxC hold/resume, C10 transfer (blind/consult) | new `cloud/wxc/register.rs`, `cloud/wxc/call.rs`, `cloud/locus/mod.rs::call_allowed` | Dialing an external mobile number from Anrufe rings that phone, and hold/resume works |
| voicemail | E4 voicemail list/play/delete/mark read/transcript, E2 MWI badge | new `cloud/wxc/voicemail.rs`, `ui/calls_page/voicemail.rs` | A voicemail left on the line appears within 1 min and plays in Webox |
| settings | F7 forwarding, F16 auto-decline, DND sync, F10 number block list | new `cloud/wxc/settings.rs`, `ui/settings/pages_b.rs` | Setting "Weiterleiten an +41…" in Webox shows the same forwarding in the official app's Calling settings |

### Batch 7: rare features (prio 3, pick as needed)

C11 merge, C15 park, C16/B14 pickup, C21 captions, C18 recording, C22 client-side layouts (multistream), C30 virtual background, C37 share audio, C28/C29 audio modes, C39 remote control, C41 step away, C42 FECC, C44 TCP/TLS fallback, A10/D7 device and deskphone calling, E6 AI summary.

Each acceptance test has the form "the feature visibly works against the official client on a second account". Group them by lane as above:
* `wxc/`: C11, C15, C16
* LLM / data channel: C21
* media: C30, C37, C28, C29, C44
* UI: C41, C42

---

## Open questions (resolve with a live trace of the official client before implementing)

1. Locus hold/resume for Webex calls (C9): which request, and does the far side see "on hold"?
2. Escalation to a meeting (C12): the request sequence and the meeting type created.
3. Whether reactions, raise hand and record appear in a plain 1:1 Webex call in the official app (C18, C19, C20).
4. Under DND: is the incoming call toast hidden or only silenced (B9)?
5. Whether the test account has a Webex Calling line (section 3).
