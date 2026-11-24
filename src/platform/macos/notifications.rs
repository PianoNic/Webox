//! Notification Center through `UNUserNotificationCenter`: banners under Webox's own name and icon, a click that opens the space,
//! buttons (Annehmen / Ablehnen for calls, the reminders' buttons). That API only works inside an app bundle (`Webox.app`):
//! a bare binary (`cargo run`) falls back to `osascript` banners (shown as "Script Editor", no click actions).
//! Answers (clicks, buttons) are queued here and taken by the app once per frame (`take`).
#![allow(unused_unsafe)]
use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction, UNNotificationActionOptions,
    UNNotificationCategory, UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

/// A click on a notification (`action` "default") or one of its buttons (`action` = the button's id).
#[derive(Clone, Debug, PartialEq)]
pub struct Answer { pub id: String, pub action: String }

static ANSWERS: Mutex<Vec<Answer>> = Mutex::new(Vec::new());
static CTX: Mutex<Option<eframe::egui::Context>> = Mutex::new(None);
/// Categories by id (the set of buttons): `setNotificationCategories` replaces the whole set, so every one is kept here.
static CATEGORIES: Mutex<BTreeMap<String, Vec<(String, String)>>> = Mutex::new(BTreeMap::new());

pub fn take() -> Vec<Answer> { std::mem::take(&mut *ANSWERS.lock().unwrap()) }

/// The id UN uses for a plain click on the banner.
const DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WeboxNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _c: &UNUserNotificationCenter, r: &UNNotificationResponse, done: &DynBlock<dyn Fn()>) {
            let (id, action) = unsafe { (r.notification().request().identifier().to_string(), r.actionIdentifier().to_string()) };
            let action = if action == DEFAULT_ACTION { "default".to_string() } else { action };
            ANSWERS.lock().unwrap().push(Answer { id, action });
            if let Some(c) = CTX.lock().unwrap().as_ref() { c.request_repaint(); }
            done.call(());
        }
        /// Also as a banner while Webox is in front (the app only notifies when its window isn't).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _c: &UNUserNotificationCenter, _n: &UNNotification, done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::Sound | UNNotificationPresentationOptions::List,));
        }
    }
);

/// Running from `Webox.app` (a bundle with an identifier): only then may `UNUserNotificationCenter` be touched (it raises an
/// Objective-C exception, which aborts, in a bare binary).
pub fn bundled() -> bool {
    static B: OnceLock<bool> = OnceLock::new();
    *B.get_or_init(|| unsafe {
        let b = NSBundle::mainBundle();
        b.bundleIdentifier().is_some() && b.bundlePath().to_string().ends_with(".app")
    })
}

fn center() -> Option<Retained<UNUserNotificationCenter>> {
    if !bundled() { return None; }
    static DELEGATE: OnceLock<usize> = OnceLock::new();
    let c = unsafe { UNUserNotificationCenter::currentNotificationCenter() };
    DELEGATE.get_or_init(|| {
        let d: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
        unsafe { c.setDelegate(Some(ProtocolObject::from_ref(&*d))) };
        let ptr = Retained::into_raw(d) as usize; // the center keeps only a weak reference: the delegate lives for the whole run
        // Asks once (the first time macOS shows its prompt); a "no" leaves the banners silent, nothing else changes.
        let opts = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge;
        let cb = RcBlock::new(|_ok: Bool, _err: *mut NSError| {});
        unsafe { c.requestAuthorizationWithOptions_completionHandler(opts, &cb) };
        ptr
    });
    Some(c)
}

/// Starts the delegate early (a click on a banner from before a restart reaches the app too).
pub fn init(ctx: &eframe::egui::Context) {
    *CTX.lock().unwrap() = Some(ctx.clone());
    let _ = center();
}

/// Registers the button set `buttons` ((id, label)) as a category and returns its id ("" without buttons).
fn category(c: &UNUserNotificationCenter, buttons: &[(&str, &str)]) -> String {
    if buttons.is_empty() { return String::new(); }
    let id = format!("webox:{}", buttons.iter().map(|b| b.0).collect::<Vec<_>>().join(","));
    let mut all = CATEGORIES.lock().unwrap();
    if all.contains_key(&id) { return id; }
    all.insert(id.clone(), buttons.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect());
    let cats: Vec<Retained<UNNotificationCategory>> = all.iter().map(|(cid, bs)| unsafe {
        let acts: Vec<Retained<UNNotificationAction>> = bs.iter().map(|(a, l)| {
            let opt = if a == "decline" { UNNotificationActionOptions::Destructive } else { UNNotificationActionOptions::Foreground };
            UNNotificationAction::actionWithIdentifier_title_options(&NSString::from_str(a), &NSString::from_str(l), opt)
        }).collect();
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(&NSString::from_str(cid), &NSArray::from_retained_slice(&acts),
            &NSArray::new(), UNNotificationCategoryOptions::empty())
    }).collect();
    unsafe { c.setNotificationCategories(&NSSet::from_retained_slice(&cats)) };
    id
}

/// Shows a banner. `id` names it (the same id replaces an older banner; answers carry it); `buttons` (id, label).
/// Returns false when it went the osascript way (no answers will come).
pub fn send(id: &str, title: &str, body: &str, buttons: &[(&str, &str)], sound: bool) -> bool {
    let Some(c) = center() else { super::notify(title, body); return false };
    unsafe {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        if sound { content.setSound(Some(&UNNotificationSound::defaultSound())); }
        let cat = category(&c, buttons);
        if !cat.is_empty() { content.setCategoryIdentifier(&NSString::from_str(&cat)); }
        let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(id), &content, None);
        c.addNotificationRequest_withCompletionHandler(&req, None);
    }
    true
}

/// Takes a banner away (the call stopped ringing).
pub fn remove(id: &str) {
    let Some(c) = center() else { return };
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    unsafe {
        c.removeDeliveredNotificationsWithIdentifiers(&ids);
        c.removePendingNotificationRequestsWithIdentifiers(&ids);
    }
}

