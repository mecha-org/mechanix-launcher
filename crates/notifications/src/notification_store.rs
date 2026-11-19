use mecha_wayland::prelude::*;
use std::collections::HashMap;
use std::time::SystemTime;

use crate::notify::Notification;

type AppName = String;

#[derive(Default)]
pub struct NotificationStore {
    pub entries: HashMap<AppName, Vec<Entry>>,
    pub next_id: u32,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u32,
    pub received_time: SystemTime,
    pub notification: Notification,
}

impl Resource for NotificationStore {}

impl NotificationStore {
    pub fn add(&mut self, notification: Notification) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.entries
            .entry(notification.app_name.clone())
            .or_default()
            .push(Entry {
                id,
                received_time: SystemTime::now(),
                notification,
            });
        id
    }

    pub fn get(&self, id: u32) -> Option<&Entry> {
        self.entries.values().flatten().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Entry> {
        self.entries.values_mut().flatten().find(|e| e.id == id)
    }

    pub fn remove(&mut self, id: u32) -> Option<Entry> {
        let app = self
            .entries
            .iter()
            .find(|(_, group)| group.iter().any(|e| e.id == id))?
            .0
            .clone();
        let group = self.entries.get_mut(&app)?;
        let entry = group.remove(group.iter().position(|e| e.id == id)?);
        if group.is_empty() {
            self.entries.remove(&app);
        }
        Some(entry)
    }

    pub fn remove_app(&mut self, app_name: &str) -> Vec<u32> {
        self.entries
            .remove(app_name)
            .into_iter()
            .flatten()
            .map(|e| e.id)
            .collect()
    }

    pub fn drain_ids(&mut self) -> Vec<u32> {
        self.entries
            .drain()
            .flat_map(|(_, group)| group)
            .map(|e| e.id)
            .collect()
    }
}
