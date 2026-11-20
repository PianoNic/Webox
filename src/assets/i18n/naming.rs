//! The app's name in the UI. This application is **Webox**, and it talks to the **Webex** service.
//!
//! Rule (docs/naming.md): "Webex" is replaced by [`APP_NAME`] only where it names THIS APPLICATION (its windows,
//! settings, autostart, restart/quit, look, audio processing, keyboard shortcuts ...). It stays "Webex" where it names
//! the service, cloud, account, site, devices or product features (Webex-Konto, Webex Calling, Webex-Site, Webex
//! Meetings, "Zu Webex einladen", sign-in, help centre, legal notices) and in anything the server sends (space titles).
//!
//! Language-pack strings go through two reviewed lists, applied by [`brand`] inside `tr`/`trn` (so every language, the
//! English source and the pack fallback get the same treatment; the pack itself is never blanket-rewritten):
//! * [`APP_PLACEHOLDER_SOURCES`]: sources whose `{{APP_NAME}}` means the app -> [`APP_NAME`]. Every other
//!   `{{APP_NAME}}` is filled with [`SERVICE_NAME`] (most of them are "{{APP_NAME}} account / site / service / meeting").
//! * [`APP_KEYS`]: (context, source) pairs where the translators wrote "Webex" instead of the placeholder but mean
//!   the app; "Webex" in their text becomes [`APP_NAME`]. A `QObject` entry matches from any context (the pack's shared
//!   strings are reached through the `QObject` fallback).
//!
//! Hard-coded UI literals use [`app_name!`](crate::app_name) (a literal, so it also works in `concat!` and `const`s).
use std::borrow::Cow;

/// `app_name!()` = `"Webox"` as a literal: `concat!(app_name!(), " zusammen mit meinem Computer starten")`.
#[macro_export]
macro_rules! app_name {
    () => {
        "Webox"
    };
}

/// The name of this application, everywhere the UI names the app.
pub const APP_NAME: &str = app_name!();
/// The name of the service/cloud/account the app connects to (kept as is).
pub const SERVICE_NAME: &str = "Webex";

/// Pack sources whose `{{APP_NAME}}` names this application (all in the shared `QObject` context).
pub const APP_PLACEHOLDER_SOURCES: &[&str] = &[
    "{{APP_NAME}}",
    "Quit {{APP_NAME}}",
    "Exit {{APP_NAME}}",
    "Restart {{APP_NAME}}",
    "Restart {{APP_NAME}} App",
    "Restart {{APP_NAME}} to apply the changes.",
    "Restart {{APP_NAME}} to apply this change.",
    "Relaunch your {{APP_NAME}} App to take effect.",
    "To complete your configuration, restart {{APP_NAME}} App to continue.",
    "The camera is not responding. Restart {{APP_NAME}} and try again.",
    "Start {{APP_NAME}} when my computer starts",
    "Show {{APP_NAME}} Window",
    "Show my {{APP_NAME}} app",
    "Choose how you want {{APP_NAME}} to look.",
    "{{APP_NAME}} uses artificial intelligence to enhance audio",
    "{{APP_NAME}} uses artificial intelligence to enhance audio.",
    "{{APP_NAME}} uses artificial intelligence to enhance audio. <a href='https://cisco.com/go/smart-audio-settings'>Learn more</a>",
    "Require biometric authentication to unlock {{APP_NAME}}.",
    "This version of {{APP_NAME}} is out of date, and a new version must be installed.",
    "Select <b>Esc</b> to show {{APP_NAME}} controls and panels again",
    "Select <b>Esc</b> or <b>{{0}}</b> to show {{APP_NAME}} controls and panels again",
    "Share your {{APP_NAME}} app, including the meeting window and all floating windows",
    "Sending the voice message, please keep {{APP_NAME}} APP in the front.",
    "Logs provide details about your system settings and {{APP_NAME}} experience to help us find and fix issues.",
    "Your administrator has requested your {{APP_NAME}} log files. Please bear with us as your app may be a little slower while we transfer the information.",
    "You're using the deskphone. To make a call, switch to {{APP_NAME}} main window.",
    "{{APP_NAME}} couldn\u{2019}t get the information needed to sign you in. Return to the start page and try again.",
    "{{APP_NAME}} can't detect nearby devices because Audio enhancements are turned on in Windows. Turn off Audio enhancements under Windows Settings.",
    "When <b>Automatic</b> is selected, {{APP_NAME}} determines whether to bypass Windows built-in audio signal processing based on what works best for your device. You can also select <b>Use</b> to use Windows audio signal processing or select <b>Bypass</b> to bypass it and receive the \"raw\" audio stream from the audio driver.",
    "You can allow {{APP_NAME}} to automatically adjust the brightness, contrast, and color saturation of your self-view video, or you can manually adjust those settings for your video.",
    "When this option is checked, {{APP_NAME}} uses hardware acceleration as needed to reduce CPU usage during content sharing. This option has no effect on shared content that is encoded using the AV1 codec.",
    "When this option is on, {{APP_NAME}} improves video quality by increasing the video resolution limits for larger or maximized video layout windows on high-resolution monitors.",
    "List item {{0}} of {{1}} in a menu. Turn off {{2}} to stop it from running in {{APP_NAME}}. You will not be able to use the app, and will stop receiving all app notifications.",
    "You are currently on third party web content inside a {{APP_NAME}} modal/dialog window. This content is governed by {{0}}'s Terms and Conditions. Press ENTER to interact with the content or ESC to exit it.",
    "You are currently on third party web content inside of {{APP_NAME}}. This content is governed by {{0}}\u{2019}s Terms and Conditions.",
    "You are currently on third party web content inside of {{APP_NAME}}. This content is governed by {{0}}\u{2019}s Terms and Conditions. Press ENTER to start interacting with the content. Once in the web content, press ESC to exit and return to {{APP_NAME}} controls.",
    "You are currently on third party web content inside of {{APP_NAME}}. This content is not governed by Cisco\u{2019}s Terms and Conditions. Press ENTER to start interacting with the content. Once in the web content, press ESC to exit and return to {{APP_NAME}} controls",
];

/// (context, source) of pack strings with a hard-coded "Webex" that names this application.
pub const APP_KEYS: &[(&str, &str)] = &[
    ("GeneralTabWidget", "Start Webex when my computer starts"),
    ("GeneralTabWidget", "Connect Webex to Microsoft Outlook"),
    ("GeneralTabWidget", "Choose what you\u{2019}d like to see first every time Webex opens. "),
    ("ThemeSelectorWidget", "Choose how you want Webex to look."),
    ("SmartAudioTabWidget", "Webex uses artificial intelligence to enhance audio. <a href=\"https://cisco.com/go/smart-audio-settings\">Learn more</a>"),
    ("KeyboardShortcutsView", "Global lets you use Webex shortcuts when the app is minimized or in the background. Disabled shortcuts cannot be made global."),
    ("SettingsKeyboardShortcutsPage", "Global lets you use Webex shortcuts when the app is minimized or in the background. Disabled shortcuts cannot be made global."),
    ("SettingsMeetingGeneralPage", "Allow Webex to show others your <b>Stepped away</b> status, mute your microphone, turn off your video, and replace your video with a hot beverage icon when you're away from your camera for a while."),
    ("CallsTabWidget", "Before you call, highlight the number in the Webex app to preview."),
    ("SettingsCallingGeneralController", "Before you call, highlight the number in the Webex app to preview."),
    ("QuietHoursViewController", "These are based on your Local time zone set through Webex Settings."),
    ("SettingsNotificationsController", "These are based on your Local time zone set through Webex Settings."),
    ("MigrationFromJabberSuccessWidget", "Restart Webex"),
    ("PopupHealthCheckVdi", "New VDI plugin version ( %1 ) is available. Please install the update and restart Webex by clicking Update Ready button."),
    ("PopupHealthCheckVdi", "New VDI plugin version (%1) is available. Please install the update and restart Webex by clicking Update Ready button."),
    ("FloatingVideoContainer", "To move this window, use the arrow keys. To navigate between this window and other Webex windows, use %1."),
    ("FloatingVideoContainer", "Webex shared content view"),
    ("FloatingVideoContainer", "Webex video view"),
    ("ImmersiveLocalVideo", "Webex presenter floating window, panel"),
    ("MultitaskingContainer", "Webex floating %1 window. %2 Use Ctrl + Shift + Alt+ S to navigate to multitasking floating window. %3"),
    ("CCByMoveToMeetingWidget", "Close Webex modal"),
    ("AppShareController", "Share your Webex app, including the meeting window and all floating windows"),
    ("ScreenAppTabController", "Share your Webex app, including the meeting window and all floating windows"),
    ("RemoteShareView", "Webex has automatically dimmed shared content and video with flashing images or high-contrast visual patterns."),
    // Window titles of the app's own dialogs (the title bar shows the app name).
    ("CxBasicTitleBar", "Webex"),
    ("ActivationCodeDialog", "Webex"),
    ("DeviceActivationCodeDialog", "Webex"),
    ("RenameWhiteboardDialog", "Webex"),
    ("ShareMultiMeetingDialog", "Webex"),
    ("MeetingRequireForceUpgrade", "Webex"),
    ("QObject", "Open Webex Settings"),
    ("QObject", "Keep Webex open. Uploading logs."),
    ("QObject", "We're having a problem connecting you to the meeting. Restart Webex and try again. If the problem persists, contact your administrator."),
    ("QObject", "All the files you stored during your use of Webex will be saved at the following location. This setting is for this computer only."),
    ("QObject", "Allow Webex to see and share your Focus status"),
    ("QObject", "Allow Webex to see and share system Focus status. Click it to open system settings window."),
    ("QObject", "Webex is unable to optimize external audio in this call due to high CPU usage"),
    ("QObject", "Your version of Webex doesn't support this webinar."),
    ("QObject", "Overall Webex App CPU Usage"),
    ("QObject", "Overall Webex App Memory Usage"),
    ("QObject", "Webex Main Process CPU Usage"),
    ("QObject", "Webex Main Process Memory Usage"),
    ("QObject", "Webex Media Process CPU Usage"),
    ("QObject", "Webex Media Process Memory Usage"),
    ("QObject", "Webex ML Process CPU Usage"),
    ("QObject", "Webex ML Process Memory Usage"),
    ("QObject", "Webex Share Process CPU Usage"),
    ("QObject", "Webex Share Process Memory Usage"),
];

/// Whether `{{APP_NAME}}` in `source` names the app (else the service).
pub fn placeholder_is_app(source: &str) -> bool {
    APP_PLACEHOLDER_SOURCES.contains(&source)
}

/// Whether the hard-coded "Webex" of (context, source) names the app.
pub fn is_app_key(context: &str, source: &str) -> bool {
    APP_KEYS.iter().any(|&(c, s)| s == source && (c == context || c == "QObject"))
}

/// The name `{{APP_NAME}}` stands for in `source`.
pub fn placeholder_value(source: &str) -> &'static str {
    if placeholder_is_app(source) { APP_NAME } else { SERVICE_NAME }
}

/// Apply the naming rule to the text `tr` found for (context, source).
pub fn brand<'a>(context: &str, source: &str, text: Cow<'a, str>) -> Cow<'a, str> {
    let mut text = text;
    if text.contains("{{APP_NAME}}") {
        text = Cow::Owned(text.replace("{{APP_NAME}}", placeholder_value(source)));
    }
    if text.contains(SERVICE_NAME) && is_app_key(context, source) {
        text = Cow::Owned(text.replace(SERVICE_NAME, APP_NAME));
    }
    text
}
