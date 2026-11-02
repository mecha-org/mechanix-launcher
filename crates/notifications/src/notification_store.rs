use mecha_wayland::prelude::*;
use std::time::SystemTime;

use crate::{control::Control, notify::Notification};

#[derive(Default)]
pub struct NotificationStore {
    pub entries: Vec<Entry>,
    pub next_id: u32,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u32,
    pub received_time: SystemTime,
    pub notification: Notification,
    pub control: Option<Control>,
}

impl Resource for NotificationStore {}
