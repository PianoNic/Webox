# The app's name: Webox, not Webex

This application is **Webox**. It connects to the **Webex** service. The UI uses "Webox" wherever a text names
*this application*: its windows, settings, autostart, quit/restart, look, audio processing and keyboard shortcuts.
"Webex" stays wherever a text names the *service*: the cloud, the account, the meeting site, Webex devices, product
features (Webex Calling, Webex Meetings, Webex Assistant, Webex-Whiteboard ...), sign-in, the help centre, legal
notices, or anything the server sends (space titles such as "Webex-Bereich für ..." are shown exactly as sent).

## Where the name comes from

* `src/assets/i18n/naming.rs`: `APP_NAME = "Webox"`, `SERVICE_NAME = "Webex"`, and the macro `app_name!()`. It
  expands to the literal `"Webox"`, so you can write `concat!(crate::app_name!(), " zusammen mit meinem Computer starten")`
  in `const`s and static tables.
* Language-pack strings: `i18n::tr` / `trn` (and the `tr!` / `trq!` / `trn!` macros) pass each result through
  `naming::brand(context, source, text)`:
  * `{{APP_NAME}}` becomes **Webox** if the source is in `APP_PLACEHOLDER_SOURCES` (a reviewed list).
    Every other `{{APP_NAME}}` becomes **Webex**, because most of them are "{{APP_NAME}} account / site / service /
    meeting".
  * Some strings have "Webex" written in instead of the placeholder. If the (context, source) pair is in `APP_KEYS`
    (a reviewed list), that "Webex" becomes **Webox**. A `QObject` entry matches from any context.
  * The pack itself is not rewritten as a whole. The rule applies to every language, to the English source and to
    the pack fallback.
* `ui::widgets::tr` returns the raw pack text. Its one placeholder user (`header_plus::label`) calls
  `i18n::brand` itself.
* Qt `%1` app names: `ApplicationMenuController` "Exit %1" is filled with `i18n::APP_NAME`.

Tests (`src/assets/i18n/tests.rs`, `app_name_*`):
* With English sources and with the German pack, the app keys render "Webox" and the service keys keep "Webex".
* Every entry of both lists is a real key of the German pack. The exceptions are entries dropped by `gen_i18n.py`
  because their German text equals the source: the plain "Webex" window titles, `{{APP_NAME}}`, and the
  untranslated smart-audio link line.

## Changed strings (key → old → new)

### Hard-coded UI literals

| Where | Pack key it mirrors | Old | New |
|---|---|---|---|
| Settings › Allgemein › Anmelden (`settings/pages_a.rs`, search index `settings/nav.rs`) | GeneralTabWidget "Start Webex when my computer starts" | Webex zusammen mit meinem Computer starten | Webox zusammen mit meinem Computer starten |
| Settings › Allgemein › Outlook-Einstellungen (pages_a, nav) | GeneralTabWidget "Connect Webex to Microsoft Outlook" | Verbindung von Webex mit Microsoft Outlook herstellen | Verbindung von Webox mit Microsoft Outlook herstellen |
| Settings › Allgemein › Zielbildschirm (pages_a) | GeneralTabWidget "Choose what you'd like to see first every time Webex opens." | … wenn Webex geöffnet wird. | … wenn Webox geöffnet wird. |
| Settings search description for Zielbildschirm (nav) | (clone text) | Was beim Öffnen von Webex zuerst angezeigt wird | Was beim Öffnen von Webox zuerst angezeigt wird |
| Settings › Erscheinungsbild › Themen (pages_a, nav) | QObject "Choose how you want {{APP_NAME}} to look." | Legen Sie fest, wie Webex aussehen soll. | Legen Sie fest, wie Webox aussehen soll. |
| Settings › Smart Audio (pages_a) | QObject "{{APP_NAME}} uses artificial intelligence to enhance audio." | Webex verwendet künstliche Intelligenz, um die Audioqualität zu verbessern. | Webox verwendet künstliche Intelligenz, … |
| Settings › Benachrichtigungen › Ruhezeiten (pages_b) | SettingsNotificationsController "These are based on your Local time zone set through Webex Settings." | … in den Webex-Einstellungen festgelegt haben. | … in den Webox-Einstellungen festgelegt haben. |
| Settings › Meetings › Meeting vorübergehend verlassen (pages_b) | SettingsMeetingGeneralPage "Allow Webex to show others your Stepped away status …" | Erlauben Sie Webex, anderen Ihren Status Abwesend anzuzeigen, … | Erlauben Sie Webox, … |
| Settings › Anrufe › Für Anruf markieren (pages_b) | SettingsCallingGeneralController "Before you call, highlight the number in the Webex app to preview." | … in der Webex-App, um eine Vorschau anzuzeigen. | … in der Webox-App, … |
| Settings › Tastenkombinationen info (`settings_shortcuts.rs`) | SettingsKeyboardShortcutsPage "Global lets you use Webex shortcuts …" | Mit "Global" können Sie Webex-Tastenkombinationen verwenden, … | … Webox-Tastenkombinationen … |
| Shortcut name OpenPreferences (`shortcuts/table.rs`) | QObject "Open Webex Settings" | Webex-Einstellungen öffnen | Webox-Einstellungen öffnen |
| Status dialog window title (`profile_menu.rs`) | (window title = app name) | Webex | Webox (`APP_NAME`) |
| Profile menu "Exit %1" (`profile_menu.rs`) | ApplicationMenuController "Exit %1" | Webox literal | `APP_NAME` (same text: "Webox beenden") |

### Pack strings, `{{APP_NAME}}` → Webox (`APP_PLACEHOLDER_SOURCES`, all QObject)

German shown; before this change they were unfilled, or filled with "Webex" where the clone used them.

| Source | New (de) |
|---|---|
| {{APP_NAME}} | Webox |
| Quit {{APP_NAME}} / Exit {{APP_NAME}} | Webox beenden |
| Restart {{APP_NAME}} / Restart {{APP_NAME}} App | Webox neu starten / Webox-App neu starten |
| Restart {{APP_NAME}} to apply the changes. / … this change. | Starten Sie Webox neu, um die Änderung(en) zu übernehmen. |
| Relaunch your {{APP_NAME}} App to take effect. | Starten Sie Ihre Webox-App neu, … |
| To complete your configuration, restart {{APP_NAME}} App to continue. | … starten Sie die Webox-App neu, … |
| The camera is not responding. Restart {{APP_NAME}} and try again. | … Starten Sie Webox neu … |
| Start {{APP_NAME}} when my computer starts | Webox mit meinem Computer starten |
| Show {{APP_NAME}} Window / Show my {{APP_NAME}} app | Webox-Fenster anzeigen / Meine Webox-App anzeigen |
| Choose how you want {{APP_NAME}} to look. | Legen Sie fest, wie Webox aussehen soll. |
| {{APP_NAME}} uses artificial intelligence to enhance audio (3 variants) | Webox verwendet künstliche Intelligenz, … |
| Require biometric authentication to unlock {{APP_NAME}}. | Für das Entsperren von Webox … |
| This version of {{APP_NAME}} is out of date, … | Diese Version von Webox ist veraltet … |
| Select <b>Esc</b> (or {{0}}) to show {{APP_NAME}} controls and panels again | … die Webox-Steuerelemente … |
| Share your {{APP_NAME}} app, including the meeting window and all floating windows | Ihre Webox-App … freigeben |
| Sending the voice message, please keep {{APP_NAME}} APP in the front. | … die Webox-App im Vordergrund. |
| Logs provide details about your system settings and {{APP_NAME}} experience … | … Ihr Webox-Erlebnis … |
| Your administrator has requested your {{APP_NAME}} log files. … | … Ihre Webox-Protokolldateien … |
| You're using the deskphone. To make a call, switch to {{APP_NAME}} main window. | … zum Webox-Hauptfenster … |
| {{APP_NAME}} couldn't get the information needed to sign you in. … | Webox konnte die … nicht abrufen. … |
| {{APP_NAME}} can't detect nearby devices because Audio enhancements are turned on in Windows. … | Webox kann keine Geräte … |
| When <b>Automatic</b> is selected, {{APP_NAME}} determines whether to bypass Windows … | … legt Webox fest, … |
| You can allow {{APP_NAME}} to automatically adjust the brightness, … | … dass Webox die Helligkeit … |
| When this option is checked, {{APP_NAME}} uses hardware acceleration … | … verwendet Webox die Hardware-Beschleunigung … |
| When this option is on, {{APP_NAME}} improves video quality … | … optimiert Webox die Videoqualität … |
| List item {{0}} of {{1}} in a menu. Turn off {{2}} to stop it from running in {{APP_NAME}}. … | … in Webox … |
| You are currently on third party web content inside (a / of) {{APP_NAME}} … (4 variants) | … innerhalb von Webox … |

### Pack strings with a hard-coded "Webex" → Webox (`APP_KEYS`)

| Context | Source | Old (de) → New (de) |
|---|---|---|
| GeneralTabWidget | Start Webex when my computer starts | Webex zusammen mit meinem Computer starten → Webox … |
| GeneralTabWidget | Connect Webex to Microsoft Outlook | Verbindung von Webex mit Microsoft Outlook herstellen → … Webox … |
| GeneralTabWidget | Choose what you'd like to see first every time Webex opens. | … wenn Webex geöffnet wird. → … Webox … |
| ThemeSelectorWidget | Choose how you want Webex to look. | Wählen Sie aus, wie Webex aussehen soll. → … Webox … |
| SmartAudioTabWidget | Webex uses artificial intelligence to enhance audio. <a …>Learn more</a> | Webex verwendet künstliche Intelligenz, … → Webox … |
| KeyboardShortcutsView, SettingsKeyboardShortcutsPage | Global lets you use Webex shortcuts … | … Webex-Tastenkombinationen … → … Webox-Tastenkombinationen … |
| SettingsMeetingGeneralPage | Allow Webex to show others your <b>Stepped away</b> status, … | Erlauben Sie Webex, … → Erlauben Sie Webox, … |
| CallsTabWidget, SettingsCallingGeneralController | Before you call, highlight the number in the Webex app to preview. | … in der Webex-App … → … Webox-App … |
| QuietHoursViewController, SettingsNotificationsController | These are based on your Local time zone set through Webex Settings. | … Webex-Einstellungen … → … Webox-Einstellungen … |
| MigrationFromJabberSuccessWidget | Restart Webex | Webex neu starten → Webox neu starten |
| PopupHealthCheckVdi | New VDI plugin version (%1) … restart Webex … (2 variants) | … starten Sie Webex neu … → … Webox … |
| FloatingVideoContainer | To move this window, … other Webex windows, use %1. | … anderen Webex-Fenstern … → … Webox-Fenstern … |
| FloatingVideoContainer | Webex shared content view / Webex video view | Webex-Ansicht von freigegebenen Inhalten / Webex-Videoansicht → Webox-… |
| ImmersiveLocalVideo | Webex presenter floating window, panel | Unverankertes Webex-Moderatorfenster, Bereich → … Webox-… |
| MultitaskingContainer | Webex floating %1 window. … | Schwebendes Webex-%1-Fenster. … → … Webox-… |
| CCByMoveToMeetingWidget | Close Webex modal | Modales Webex-Fenster schließen → Modales Webox-Fenster schließen |
| AppShareController, ScreenAppTabController | Share your Webex app, including the meeting window and all floating windows | Webex-App … freigeben → Webox-App … |
| RemoteShareView | Webex has automatically dimmed shared content … | Webex hat … gedimmt → Webox hat … |
| CxBasicTitleBar, ActivationCodeDialog, DeviceActivationCodeDialog, RenameWhiteboardDialog, ShareMultiMeetingDialog, MeetingRequireForceUpgrade | Webex (window title) | Webex → Webox |
| QObject | Open Webex Settings | Webex-Einstellungen öffnen → Webox-Einstellungen öffnen |
| QObject | Keep Webex open. Uploading logs. | Webex offen halten. … → Webox offen halten. … |
| QObject | We're having a problem connecting you to the meeting. Restart Webex and try again. … | … Starten Sie Webex neu … → … Webox … |
| QObject | All the files you stored during your use of Webex will be saved at the following location. … | … Nutzung von Webex … → … Webox … |
| QObject | Allow Webex to see and share your Focus status / … system Focus status. … | Webex das Anzeigen … erlauben → Webox … |
| QObject | Webex is unable to optimize external audio in this call due to high CPU usage | Webex kann … nicht optimieren. → Webox … |
| QObject | Your version of Webex doesn't support this webinar. | Ihre Version von Webex … → … Webox … |
| QObject | Overall Webex App CPU / Memory Usage | … der Webex-App (insgesamt) → … der Webox-App … |
| QObject | Webex Main / Media / ML / Share Process CPU / Memory Usage | … des Webex-Hauptprozesses … → … Webox-… |

### Already Webox (checked, unchanged)

* Main window title and eframe app id: `main.rs` "Webox", `with_app_id("webox")`.
* Tray tooltip: `platform/tray/mod.rs` `tooltip(0)` = "Webox".
* Toast and tray-menu window titles: "Webox toast", "Webox tray menu" (`platform/notify/mod.rs`).
* Incoming-call toast, call window and call dock titles: "Webox - Eingehender Anruf", "Webox - Anruf", "Webox - Anruf-Dock".
* About dialog: "Info über Webox" (`ui/credits.rs`, rail menu).
* Linux `.desktop` file (`Name=Webox`), Nix packages and modules, flake. There is no Windows installer in the repo;
  the binary is `webox`.

## Kept "Webex" (on purpose)

| Where | Text | Why |
|---|---|---|
| Header + menu, Invite page (`header_plus.rs`, `invite.rs`) | Zu Webex einladen (QObject "Invite to {{APP_NAME}}") | Invites a person to the service |
| Welcome message tooltip (`welcome.rs`) | Weitere Informationen zur Webex-Verschlüsselung | The service's encryption |
| Personal-room error toast (`meetcall.rs`) | … von Ihrer Webex-Site … | The meeting site |
| Settings › Allgemein / Datenschutz | … dass Sie in Webex aktiv sind … (2 strings) | Presence on the service, seen by others in any Webex client |
| Settings › Multitasking | Mein(en) Webex-Anruf oder -Meeting … | A Webex call or meeting (service) |
| Settings › Meetings aus dem Browser | Ersetzt den Webex-Browser-Helper. | Name of an official product |
| Settings › Meetings | Webex-Site-Meetings, Einstellungen der Webex-Seite | The meeting site |
| Settings › Anruf-Dock | Webex Calling-Funktionen | Product name |
| Calls page (`calls_page`) | … eine andere Person anzurufen, die Webex verwendet … | People on the service |
| Meetings › Kalender auswählen (`meetings_page.rs`) | Wählen Sie einen Kalender, der mit Webex verwendet werden soll. | The calendar is linked to the Webex account (OAuth on the Webex settings site) |
| Loading banner (`loading.rs`) | Der Webex-Dienst kann nicht erreicht werden … | The service |
| Blocked-people dialog (`app_dialogs.rs`) | … der Webex-Dienst wird nicht informiert. | The service (the same text already says "nur in Webox gespeichert") |
| Device pairing (`app_dialogs.rs`) | … auf dem Webex-Gerät angezeigten … Code … | Webex hardware |
| Calling errors (`cloud/locus`) | Angerufen werden können nur Webex-Adressen (E-Mail). | Service addresses |
| Notification kickers (`platform/notify/hub.rs`) | Webex-Anrufübersicht, Webex-Whiteboard | Service features |
| Sign-in (`login.rs`) | "Webex" / "Webex for Government" rows, "… the code Webex shows you …", "Complete sign-in in the Webex window" | The account and the service's sign-in page |
| Sign-in window title (Linux, `platform/webview/platform_gtk.rs`) | Webex | Shows the Webex account sign-in page; `packaging/docker/signin-test.sh` looks the window up by this name |
| Help/terms links | help.webex.com, settings.webex.com, signup.webex.com | Service websites |
| Sample data (`cloud/sample.rs`, `app/mod.rs`) | Webex-Bereich für Alex … | Stands in for server data (space titles) |
| Internal ids and log/error strings | `webex-menu*`, `webex-dialog*`, "content location is not a Webex host", `webexteams://`, icon names | Not UI text, or about service hosts |
| Pack strings outside the lists | everything with "Webex account / site / service / meeting / device / Assistant / Calling / Meetings", "By using Webex, you accept …", "Send report to Webex", "Selecting Send Feedback will send your Webex activity logs to us …", "Webex will sign you out …", "For your security, Webex requires you to sign in again …", "Update your Webex app to join this webinar" | The service, legal text, or the official app (Webox would make a false claim, e.g. that it sends logs to Cisco) |
| `{{APP_NAME}}` outside `APP_PLACEHOLDER_SOURCES` | e.g. "{{APP_NAME}} Help", "Welcome to {{APP_NAME}}", "Show {{APP_NAME}} availability in Outlook", "The network … appears to be blocking {{APP_NAME}}" | Default: the service name |

To change the rule for a string, add its key to (or remove it from) one of the two lists in `naming.rs` and to the
tables above. `app_name_german_pack` checks that every entry is a real pack key.
