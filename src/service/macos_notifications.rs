//! macOS Notification Centre delivery for alerts. The text and the retry rules are in
//! `model::alert_notification`; this is the OS call.
//!
//! `UNUserNotificationCenter` refuses to run outside a real `.app` bundle (it raises on first
//! use) and needs extra bindings, so today's backend posts through `osascript`
//! (`display notification`), which works from a bare binary. Both sit behind `Notifier`, so a
//! `UNUserNotificationCenter` backend can replace it once the app ships as a bundle. A
//! notification shown this way is attributed to Script Editor and cannot report whether the user
//! has allowed notifications; failures that are visible (osascript missing or erroring) are
//! reported so the caller's retry state machine can act on them.
//!
//! Under test, or with `DEGENVIEW_NO_NOTIFICATIONS` / `DEGENVIEW_NO_KEYCHAIN` set, nothing is
//! posted.

// Transient: the alert host wires this into `tick_alerts` next to the banner.
#![allow(dead_code)]

use crate::model::alert_notification::NotificationContent;
use crate::service::file_dialog::applescript_string;
use std::process::Command;
use std::sync::Arc;

/// The alert sound used when a notification asks for one.
pub const SOUND_NAME: &str = "Glass";

/// Whether notifications are off-limits to this process (tests, headless runs).
pub fn is_disabled() -> bool {
    cfg!(test)
        || std::env::var_os("DEGENVIEW_NO_NOTIFICATIONS").is_some()
        || std::env::var_os("DEGENVIEW_NO_KEYCHAIN").is_some()
}

pub trait Notifier: Send + Sync {
    /// Posts one notification. Blocking: call it off the UI thread (see `notify_async`).
    fn deliver(&self, content: &NotificationContent) -> Result<(), String>;
}

/// The AppleScript that posts `content`.
pub fn notification_script(content: &NotificationContent) -> String {
    let mut script = format!(
        "display notification {} with title {}",
        applescript_string(&content.body),
        applescript_string(&content.title)
    );
    if content.sound {
        script.push_str(&format!(" sound name {}", applescript_string(SOUND_NAME)));
    }
    script
}

pub struct OsaScriptNotifier;

impl Notifier for OsaScriptNotifier {
    fn deliver(&self, content: &NotificationContent) -> Result<(), String> {
        let output = Command::new("osascript")
            .arg("-e")
            .arg(notification_script(content))
            .output()
            .map_err(|error| format!("Could not run osascript: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Notification failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

/// Reports success without posting anything.
pub struct NullNotifier;

impl Notifier for NullNotifier {
    fn deliver(&self, _: &NotificationContent) -> Result<(), String> {
        Ok(())
    }
}

/// The notifier for this run: real unless notifications are disabled.
pub fn default_notifier() -> Arc<dyn Notifier> {
    if is_disabled() {
        Arc::new(NullNotifier)
    } else {
        Arc::new(OsaScriptNotifier)
    }
}

/// Posts on a worker thread and hands the outcome to `done` (also on that thread), so the
/// caller can record the delivery as succeeded or failed without blocking the UI.
pub fn notify_async(
    notifier: Arc<dyn Notifier>,
    content: NotificationContent,
    done: impl FnOnce(&NotificationContent, Result<(), String>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let outcome = notifier.deliver(&content);
        done(&content, outcome);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn content(sound: bool) -> NotificationContent {
        NotificationContent {
            title: "BTC price alert".into(),
            body: "Reached $70,000.00".into(),
            identifier: "42".into(),
            sound,
            user_info: vec![("alertID".into(), "7".into())],
        }
    }

    #[test]
    fn the_script_carries_title_body_and_an_optional_sound() {
        assert_eq!(
            notification_script(&content(false)),
            "display notification \"Reached $70,000.00\" with title \"BTC price alert\""
        );
        assert_eq!(
            notification_script(&content(true)),
            "display notification \"Reached $70,000.00\" with title \"BTC price alert\" sound name \"Glass\""
        );
    }

    #[test]
    fn quotes_and_backslashes_in_the_text_cannot_break_out_of_the_script() {
        let mut hostile = content(false);
        hostile.title = "say \"hi\" \\ then\nquit".into();
        hostile.body = "\" & (do shell script \"rm -rf ~\") & \"".into();
        let script = notification_script(&hostile);
        assert!(
            script.contains("with title \"say \\\"hi\\\" \\\\ then quit\""),
            "{script}"
        );
        assert!(
            script.contains(
                "display notification \"\\\" & (do shell script \\\"rm -rf ~\\\") & \\\"\""
            ),
            "{script}"
        );
    }

    #[test]
    fn under_test_the_default_notifier_posts_nothing() {
        assert!(is_disabled());
        assert!(default_notifier().deliver(&content(true)).is_ok());
    }

    struct Recording(Mutex<Vec<String>>, Result<(), String>);

    impl Notifier for Recording {
        fn deliver(&self, content: &NotificationContent) -> Result<(), String> {
            self.0.lock().unwrap().push(content.identifier.clone());
            self.1.clone()
        }
    }

    #[test]
    fn the_async_post_reports_back_with_the_outcome() {
        let ok = Arc::new(Recording(Mutex::new(Vec::new()), Ok(())));
        let (sender, receiver) = channel();
        let sent = sender.clone();
        notify_async(ok.clone(), content(false), move |content, outcome| {
            let _ = sent.send((content.identifier.clone(), outcome));
        });
        let (identifier, outcome) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((identifier.as_str(), outcome), ("42", Ok(())));
        assert_eq!(*ok.0.lock().unwrap(), vec!["42".to_owned()]);

        let failing = Arc::new(Recording(Mutex::new(Vec::new()), Err("denied".into())));
        notify_async(failing, content(false), move |_, outcome| {
            let _ = sender.send(("x".into(), outcome));
        });
        let (_, outcome) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(outcome, Err("denied".to_owned()));
    }
}
