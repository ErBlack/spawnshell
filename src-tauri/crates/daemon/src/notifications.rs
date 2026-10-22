use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;

use anyhow::{Context, Result, anyhow};

use super::hub::{Hub, Tab};
use super::report;

const UNTITLED: &str = "Spawnshell";
const MAX_TITLE: usize = 256;
const MAX_BODY: usize = 2048;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Notification {
    pub(super) title: Option<String>,
    pub(super) body: String,
}

fn clean(text: &str, limit: usize) -> String {
    let text: String = text
        .chars()
        .map(|letter| if letter.is_control() { ' ' } else { letter })
        .collect();
    text.trim().chars().take(limit).collect()
}

impl Notification {
    pub(super) fn new(title: Option<&str>, body: &str) -> Option<Notification> {
        let title = title
            .map(|title| clean(title, MAX_TITLE))
            .filter(|title| !title.is_empty());
        let body = clean(body, MAX_BODY);
        (title.is_some() || !body.is_empty()).then_some(Notification { title, body })
    }
}

pub(super) type TabNotification = (String, Notification);

impl Tab {
    pub(super) fn dismiss_notification(&mut self) -> bool {
        let notified = std::mem::take(&mut self.entry.notified);
        if notified {
            notifier::remove(&self.entry.id);
        }
        notified
    }
}

impl Hub {
    pub(super) fn watch_notifications(
        self: &Arc<Self>,
        notifications: mpsc::Receiver<TabNotification>,
    ) {
        let hub = Arc::clone(self);
        thread::spawn(move || {
            for (tab, notification) in notifications {
                let _ = hub.show_notification(&tab, notification);
            }
        });
    }

    pub(super) fn request_notification(
        self: &Arc<Self>,
        id: &str,
        title: Option<&str>,
        body: &str,
    ) -> Result<()> {
        let notification = Notification::new(title, body).context("the notification is empty")?;
        self.show_notification(id, notification)
    }

    fn show_notification(self: &Arc<Self>, id: &str, notification: Notification) -> Result<()> {
        let mut state = self.state.lock();
        let index = state.find(id)?;
        if self.window_open.load(Ordering::SeqCst) && state.active.as_deref() == Some(id) {
            return Ok(());
        }
        let tab = &mut state.tabs[index];
        let label = tab.label();
        tab.entry.notified = true;
        drop(state);

        let (title, subtitle) = match notification.title {
            Some(title) if title == label => (title, String::new()),
            Some(title) => (title, label),
            None if label.is_empty() => (UNTITLED.to_string(), String::new()),
            None => (label, String::new()),
        };
        let request = notifier::Notification {
            id,
            title: &title,
            subtitle: &subtitle,
            body: &notification.body,
            sound: self.config().notification_sound,
        };
        let hub = Arc::clone(self);
        notifier::post(&request, move |error| hub.notification_failed(&error));
        self.changed();
        Ok(())
    }

    fn notification_failed(&self, error: &str) {
        if self.notifications_failed.swap(true, Ordering::SeqCst) {
            return;
        }
        report(&anyhow!("can't show notifications: {error}"));
    }

    fn clear_notification(&self, id: &str) {
        let dismissed = {
            let mut state = self.state.lock();
            state
                .tabs
                .iter_mut()
                .find(|tab| tab.entry.id == id)
                .is_some_and(Tab::dismiss_notification)
        };
        if dismissed {
            self.changed();
        }
    }

    pub(super) fn activated(&self, id: &str) {
        if self.window_open.load(Ordering::SeqCst) {
            self.clear_notification(id);
        }
    }

    pub(super) fn set_window_open(&self, open: bool) {
        self.window_open.store(open, Ordering::SeqCst);
        if !open {
            return;
        }
        let active = self.state.lock().active.clone();
        if let Some(active) = active {
            self.clear_notification(&active);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_loses_control_characters_and_length() {
        let notification = Notification::new(Some(" \u{9b}Build "), "  first\u{9b}second ");
        assert_eq!(
            notification,
            Some(Notification {
                title: Some("Build".to_string()),
                body: "first second".to_string(),
            })
        );
        let long = Notification::new(None, &"x".repeat(MAX_BODY + 10)).unwrap();
        assert_eq!(long.body.len(), MAX_BODY);
        assert_eq!(Notification::new(Some(" "), "\u{9b}"), None);
    }
}
