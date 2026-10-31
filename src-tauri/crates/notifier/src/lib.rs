use std::sync::Arc;

use parking_lot::Mutex;

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "other.rs")]
mod imp;

pub use imp::{listen, post, remove, request_permission};

pub struct Notification<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub body: &'a str,
    pub sound: bool,
}

type Handler = Arc<dyn Fn(String) + Send + Sync>;

struct Clicks {
    handler: Option<Handler>,
    pending: Vec<String>,
}

static CLICKS: Mutex<Clicks> = Mutex::new(Clicks {
    handler: None,
    pending: Vec::new(),
});

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn clicked(id: String) {
    let mut clicks = CLICKS.lock();
    match clicks.handler.clone() {
        Some(handler) => {
            drop(clicks);
            handler(id);
        }
        None => clicks.pending.push(id),
    }
}

pub fn on_click(handler: impl Fn(String) + Send + Sync + 'static) {
    let handler: Handler = Arc::new(handler);
    let pending = {
        let mut clicks = CLICKS.lock();
        clicks.handler = Some(Arc::clone(&handler));
        std::mem::take(&mut clicks.pending)
    };
    for id in pending {
        handler(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_wait_for_the_handler_which_runs_outside_the_lock() {
        clicked("early".to_string());

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        on_click(move |id| {
            if id == "late" {
                clicked("nested".to_string());
            }
            sink.lock().push(id);
        });
        assert_eq!(*seen.lock(), ["early"]);

        clicked("late".to_string());
        assert_eq!(*seen.lock(), ["early", "nested", "late"]);
    }
}
