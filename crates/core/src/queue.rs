//! Per-target reminder queues. A reminder stays queued until the host that
//! delivered it into the agent's session acknowledges it — SourceFed's
//! `MonitorEventQueue` contract, applied to reminders.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::steer::Reminder;
use crate::target::Target;

/// Queued reminders kept per target, at most. Oldest are dropped.
pub const QUEUE_LIMIT_PER_TARGET: usize = 50;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct QueuedReminder {
    pub id: String,
    pub target: Target,
    pub reminder: Reminder,
    /// Unix seconds.
    pub queued_at: u64,
}

pub trait ReminderQueue: Send + Sync {
    fn enqueue(&self, target: &Target, reminder: Reminder) -> Result<QueuedReminder, String>;
    fn read(&self, target: &Target) -> Result<Vec<QueuedReminder>, String>;
    fn acknowledge(&self, target: &Target, ids: &[String]) -> Result<(), String>;
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn queued(target: &Target, reminder: Reminder, at: u64) -> QueuedReminder {
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);

    QueuedReminder {
        id: format!("reminder-{at}-{sequence}"),
        target: target.clone(),
        reminder,
        queued_at: at,
    }
}

fn push_bounded(events: &mut Vec<QueuedReminder>, item: QueuedReminder) -> QueuedReminder {
    let target = item.target.clone();
    events.push(item.clone());

    let mut per_target = events.iter().filter(|e| e.target == target).count();

    while per_target > QUEUE_LIMIT_PER_TARGET {
        if let Some(index) = events.iter().position(|e| e.target == target) {
            events.remove(index);
        }

        per_target -= 1;
    }

    item
}

#[derive(Default)]
pub struct InMemoryReminderQueue {
    events: Mutex<Vec<QueuedReminder>>,
}

impl ReminderQueue for InMemoryReminderQueue {
    fn enqueue(&self, target: &Target, reminder: Reminder) -> Result<QueuedReminder, String> {
        let mut events = self.events.lock().map_err(|_| "queue lock poisoned")?;

        Ok(push_bounded(&mut events, queued(target, reminder, now())))
    }

    fn read(&self, target: &Target) -> Result<Vec<QueuedReminder>, String> {
        let events = self.events.lock().map_err(|_| "queue lock poisoned")?;

        Ok(events
            .iter()
            .filter(|e| &e.target == target)
            .cloned()
            .collect())
    }

    fn acknowledge(&self, target: &Target, ids: &[String]) -> Result<(), String> {
        let mut events = self.events.lock().map_err(|_| "queue lock poisoned")?;
        events.retain(|e| !(&e.target == target && ids.contains(&e.id)));

        Ok(())
    }
}

/// `reminders.json` in the state directory. Every operation loads, mutates,
/// and writes atomically under one lock; volumes are tiny.
pub struct JsonReminderQueue {
    path: PathBuf,
    lock: Mutex<()>,
}

impl JsonReminderQueue {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join("reminders.json"),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Vec<QueuedReminder>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| format!("corrupt {}: {e}", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("read {}: {error}", self.path.display())),
        }
    }

    fn save(&self, events: &[QueuedReminder]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }

        let tmp = self.path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(events).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, raw).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("rename {}: {e}", self.path.display()))
    }

    fn transact<T>(&self, op: impl FnOnce(&mut Vec<QueuedReminder>) -> T) -> Result<T, String> {
        let _guard = self.lock.lock().map_err(|_| "queue lock poisoned")?;
        let mut events = self.load()?;
        let out = op(&mut events);
        self.save(&events)?;

        Ok(out)
    }
}

impl ReminderQueue for JsonReminderQueue {
    fn enqueue(&self, target: &Target, reminder: Reminder) -> Result<QueuedReminder, String> {
        self.transact(|events| push_bounded(events, queued(target, reminder, now())))
    }

    fn read(&self, target: &Target) -> Result<Vec<QueuedReminder>, String> {
        let _guard = self.lock.lock().map_err(|_| "queue lock poisoned")?;

        Ok(self
            .load()?
            .into_iter()
            .filter(|e| &e.target == target)
            .collect())
    }

    fn acknowledge(&self, target: &Target, ids: &[String]) -> Result<(), String> {
        self.transact(|events| events.retain(|e| !(&e.target == target && ids.contains(&e.id))))
    }
}

/// Group queued reminders by target key; used by SSE fan-out.
pub fn by_target(events: Vec<QueuedReminder>) -> HashMap<String, Vec<QueuedReminder>> {
    let mut grouped: HashMap<String, Vec<QueuedReminder>> = HashMap::new();

    for event in events {
        grouped.entry(event.target.key()).or_default().push(event);
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steer::Urgency;

    fn reminder(rule: &str) -> Reminder {
        Reminder {
            agent_id: "a".into(),
            rule_id: rule.into(),
            urgency: Urgency::Routine,
            text: "t".into(),
        }
    }

    fn exercise(queue: &dyn ReminderQueue) {
        let t1 = Target::new("cli", "one");
        let t2 = Target::new("cli", "two");

        let first = queue.enqueue(&t1, reminder("r1")).expect("enqueue");
        queue.enqueue(&t2, reminder("r2")).expect("enqueue");

        assert_eq!(queue.read(&t1).expect("read").len(), 1);
        assert_eq!(queue.read(&t2).expect("read")[0].reminder.rule_id, "r2");

        queue.acknowledge(&t1, &[first.id]).expect("ack");

        assert!(queue.read(&t1).expect("read").is_empty());
        assert_eq!(queue.read(&t2).expect("read").len(), 1);
    }

    #[test]
    fn in_memory_queue_is_target_scoped_and_acknowledgeable() {
        exercise(&InMemoryReminderQueue::default());
    }

    #[test]
    fn json_queue_persists_across_instances() {
        let dir = std::env::temp_dir().join(format!("chauffeur-queue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        exercise(&JsonReminderQueue::new(&dir));

        let reopened = JsonReminderQueue::new(&dir);
        assert_eq!(
            reopened
                .read(&Target::new("cli", "two"))
                .expect("read")
                .len(),
            1
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_is_bounded_per_target() {
        let queue = InMemoryReminderQueue::default();
        let target = Target::new("cli", "x");

        for index in 0..(QUEUE_LIMIT_PER_TARGET + 5) {
            queue
                .enqueue(&target, reminder(&format!("r{index}")))
                .expect("enqueue");
        }

        let kept = queue.read(&target).expect("read");
        assert_eq!(kept.len(), QUEUE_LIMIT_PER_TARGET);
        assert_eq!(
            kept.last().map(|e| e.reminder.rule_id.as_str()),
            Some("r54")
        );
    }
}
