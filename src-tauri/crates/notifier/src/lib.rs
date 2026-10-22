use std::path::Path;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use parking_lot::Mutex;

pub struct Notification<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub body: &'a str,
    pub sound: bool,
}

type Handler = Box<dyn Fn(String) + Send>;

struct Clicks {
    handler: Option<Handler>,
    pending: Vec<String>,
}

static CLICKS: Mutex<Clicks> = Mutex::new(Clicks {
    handler: None,
    pending: Vec::new(),
});

fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let bundle = NSBundle::mainBundle();
        let path = bundle.bundlePath().to_string();
        let in_app = Path::new(&path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        bundle.bundleIdentifier().is_some() && in_app
    })
}

fn center() -> Option<Retained<UNUserNotificationCenter>> {
    available().then(UNUserNotificationCenter::currentNotificationCenter)
}

fn describe(error: *mut NSError) -> Option<String> {
    unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string())
}

pub fn post(notification: &Notification, on_error: impl Fn(String) + 'static) {
    let Some(center) = center() else {
        return;
    };
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(notification.title));
    content.setSubtitle(&NSString::from_str(notification.subtitle));
    content.setBody(&NSString::from_str(notification.body));
    if notification.sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(notification.id),
        &content,
        None,
    );
    let completion = RcBlock::new(move |error: *mut NSError| {
        if let Some(error) = describe(error) {
            on_error(error);
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
}

pub fn remove(id: &str) {
    let Some(center) = center() else {
        return;
    };
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    center.removeDeliveredNotificationsWithIdentifiers(&ids);
}

pub fn request_permission(on_answer: impl Fn(bool, Option<String>) + 'static) {
    let Some(center) = center() else {
        return;
    };
    let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        on_answer(granted.as_bool(), describe(error));
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &completion,
    );
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "SpawnshellNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            clicked(response.notification().request().identifier().to_string());
            handler.call(());
        }
    }
);

fn clicked(id: String) {
    let mut clicks = CLICKS.lock();
    match &clicks.handler {
        Some(handler) => handler(id),
        None => clicks.pending.push(id),
    }
}

pub fn listen() {
    let Some(center) = center() else {
        return;
    };
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let _ = Retained::into_raw(delegate);
}

pub fn on_click(handler: impl Fn(String) + Send + 'static) {
    let mut clicks = CLICKS.lock();
    for id in std::mem::take(&mut clicks.pending) {
        handler(id);
    }
    clicks.handler = Some(Box::new(handler));
}
