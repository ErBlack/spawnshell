use crate::Notification;

pub fn post(_notification: &Notification, _on_error: impl Fn(String) + 'static) {}

pub fn remove(_id: &str) {}

pub fn request_permission(_on_answer: impl Fn(bool, Option<String>) + 'static) {}

pub fn listen() {}
