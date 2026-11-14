//! Уведомления macOS (UserNotifications) с кнопкой «Показать в Finder».
//!
//! Работают, только когда бот запущен из своего .app-бандла (FileReceiverBot.app):
//! так macOS знает, от какого приложения уведомление, и показывает его имя и иконку.
//! Ответы на нажатия приходят в основной поток, где крутится run loop (см. main.rs).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

const CATEGORY: &str = "file-saved";
const ACTION_REVEAL: &str = "reveal";

static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTER: AtomicU64 = AtomicU64::new(0);
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

/// Последняя ошибка запроса разрешения (для самопроверки)
pub fn last_error() -> String {
    LAST_ERROR.lock().unwrap().clone()
}

/// Статус разрешения, как его видит macOS (для самопроверки)
pub fn authorization_status() -> String {
    if !in_bundle() {
        return "нет бандла".into();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let block = RcBlock::new(move |settings: std::ptr::NonNull<objc2_user_notifications::UNNotificationSettings>| {
        let s = unsafe { settings.as_ref() };
        let _ = tx.send(format!(
            "authorizationStatus={:?}, alertSetting={:?}, alertStyle={:?}",
            s.authorizationStatus(),
            s.alertSetting(),
            s.alertStyle()
        ));
    });
    UNUserNotificationCenter::currentNotificationCenter().getNotificationSettingsWithCompletionHandler(&block);
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or_else(|_| "нет ответа".into())
}

/// Какой файл показать по нажатию на уведомление
fn paths() -> &'static Mutex<HashMap<String, PathBuf>> {
    static P: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

define_class!(
    // SAFETY: NSObject не требует ничего особенного от наследников, Drop не реализован.
    #[unsafe(super(NSObject))]
    #[name = "FileReceiverNotificationDelegate"]
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
            let id = response.notification().request().identifier().to_string();
            let action = response.actionIdentifier().to_string();
            // Нажатие на само уведомление или на кнопку — показываем файл в Finder
            let dismissed = action == "com.apple.UNNotificationDismissActionIdentifier";
            if let Some(path) = paths().lock().unwrap().remove(&id) {
                if !dismissed {
                    let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&path).spawn();
                }
            }
            handler.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Запущены ли мы из .app-бандла (без него UserNotifications недоступны)
pub fn in_bundle() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Настройка уведомлений. Вызывать из основного потока при старте.
pub fn init() {
    if !in_bundle() {
        log::warn!("Уведомления недоступны: бот запущен не из FileReceiverBot.app");
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();

    let reveal = UNNotificationAction::actionWithIdentifier_title_options(
        &NSString::from_str(ACTION_REVEAL),
        &NSString::from_str("Показать в Finder"),
        UNNotificationActionOptions::Foreground,
    );
    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(CATEGORY),
        &NSArray::from_retained_slice(&[reveal]),
        &NSArray::new(),
        UNNotificationCategoryOptions::CustomDismissAction,
    );
    center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));

    let delegate = Delegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // Делегат хранится «слабо» — держим его живым всё время работы
    std::mem::forget(delegate);

    let done = RcBlock::new(|granted: Bool, err: *mut NSError| {
        ENABLED.store(granted.as_bool(), Ordering::Relaxed);
        if let Some(e) = unsafe { err.as_ref() } {
            *LAST_ERROR.lock().unwrap() = format!("{} (код {})", e.localizedDescription(), e.code());
        }
        if granted.as_bool() {
            log::info!("Уведомления разрешены");
        } else {
            let reason = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
            log::warn!(
                "Уведомления запрещены ({reason}) — разрешите их: Системные настройки → Уведомления → File Receiver"
            );
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn file_saved(title: String, message: String, path: PathBuf) {
    if !in_bundle() {
        return;
    }
    if !ENABLED.load(Ordering::Relaxed) {
        log::debug!("Уведомление не показано: нет разрешения");
    }
    let id = format!("file-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    {
        let mut p = paths().lock().unwrap();
        // Не копим бесконечно: старые уведомления всё равно уже не нажмут
        if p.len() > 200 {
            p.clear();
        }
        p.insert(id.clone(), path);
    }

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&title));
    content.setBody(&NSString::from_str(&message));
    content.setCategoryIdentifier(&NSString::from_str(CATEGORY));
    content.setSound(Some(&UNNotificationSound::defaultSound()));

    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
    let done = RcBlock::new(|err: *mut NSError| {
        if let Some(e) = unsafe { err.as_ref() } {
            log::warn!("Не удалось показать уведомление: {}", e.localizedDescription());
        }
    });
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&done));
}
