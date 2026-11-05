//! The two pieces of macOS the app uses directly: an open panel to choose the `hq` program, and
//! the system notice for a helper that needs the person's OK. What either may do is decided in
//! `hq-desktop` (`chooser`, `notice`); this file only shows and posts.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSBundle, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
    UNUserNotificationCenter,
};

/// Shows the open panel and returns the file chosen, if any. Must run on the main thread.
pub fn choose_file(mtm: MainThreadMarker) -> Option<std::path::PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str("Choose the hq program")));
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URL()?;
    url.path().map(|p| std::path::PathBuf::from(p.to_string()))
}

/// Posts one notice. Does nothing when the app is not running from a bundle (a development
/// run has no identity to ask permission as), and nothing when the person has not allowed it.
pub fn post_notice(title: &str, body: &str) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &block2::RcBlock::new(|_granted, _error| {}),
    );
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let id = NSString::from_str(&format!("helper-needs-ok-{n}"));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    center.addNotificationRequest_withCompletionHandler(&request, None);
}
