//! Desktop alerts for connected sessions, with cross-process FCM deduplication.
use crate::{
    api::ApiResult,
    model::{Chat, Message},
};
use rusqlite::{params, Connection, TransactionBehavior};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub struct Tracker {
    latest: HashMap<String, Option<i64>>,
    seen: HashSet<String>,
    order: VecDeque<String>,
    started: i64,
}
impl Tracker {
    pub fn reset(&mut self, chats: &[Chat]) {
        *self = Self {
            started: chrono::Utc::now().timestamp_millis(),
            ..Default::default()
        };
        for chat in chats {
            self.latest.insert(chat.guid.clone(), chat.last_activity());
            if let Some(message) = &chat.last_message {
                self.remember(&message.guid);
            }
        }
    }
    fn remember(&mut self, guid: &str) -> bool {
        if !self.seen.insert(guid.to_owned()) {
            return false;
        }
        self.order.push_back(guid.to_owned());
        if self.order.len() > 4096 {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
    pub fn incoming(&mut self, chat: &str, message: &Message) -> bool {
        if message.guid.is_empty() || !self.remember(&message.guid) {
            return false;
        }
        let time = message.activity_timestamp();
        let newer = match self.latest.get(chat) {
            Some(previous) => time >= *previous,
            None => time.is_some_and(|time| time >= self.started),
        };
        let previous = self.latest.entry(chat.to_owned()).or_default();
        *previous = (*previous).max(time);
        newer && !message.is_from_me && message.associated_message_guid.is_none()
    }
}

pub struct Alert {
    pub scope: String,
    pub guid: String,
    pub title: String,
    pub body: String,
}

enum Command {
    Alert(Alert),
    Lease(Option<String>),
}

pub struct Desktop {
    tx: mpsc::Sender<Command>,
    errors: mpsc::Receiver<String>,
}
impl Default for Desktop {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let (errors_tx, errors) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lease = None;
            for command in rx {
                let alert = match command {
                    Command::Alert(alert) => alert,
                    Command::Lease(scope) => {
                        update_lease(lease.as_deref(), scope.as_deref());
                        lease = scope;
                        continue;
                    }
                };
                let result = deliver_once(&alert.scope, &alert.guid, || {
                    notify_rust::Notification::new().appname("BlueBubbles")
                        .summary(&alert.title).body(&alert.body).icon("app.bluebubbles.RustLinux")
                        .timeout(notify_rust::Timeout::Milliseconds(7000)).show()
                        .map(|_| ()).map_err(|_| "Desktop notifications are unavailable. Check your desktop notification service.".into())
                });
                if let Err(error) = result {
                    let _ = errors_tx.send(error);
                }
            }
            update_lease(lease.as_deref(), None);
        });
        Self { tx, errors }
    }
}
impl Desktop {
    pub fn send(&self, alert: Alert) {
        let _ = self.tx.send(Command::Alert(alert));
    }
    pub fn lease(&self, scope: Option<String>) {
        let _ = self.tx.send(Command::Lease(scope));
    }
    pub fn errors(&self) -> impl Iterator<Item = String> + '_ {
        self.errors.try_iter()
    }
}

pub fn body(text: &str) -> String {
    text.chars()
        .take(240)
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn ledger_path() -> ApiResult<PathBuf> {
    directories::ProjectDirs::from("app", "bluebubbles", "rust-linux")
        .map(|dir| {
            dir.data_local_dir()
                .join("notifications")
                .join("delivered.sqlite3")
        })
        .ok_or("Cannot locate notification storage.".into())
}

/// The transaction serializes GUI/FCM delivery. Only IDs are retained, never text
/// or credentials. Failed desktop delivery is not marked delivered.
pub fn deliver_once(
    scope: &str,
    guid: &str,
    deliver: impl FnOnce() -> ApiResult<()>,
) -> ApiResult<()> {
    deliver_at(&ledger_path()?, scope, guid, deliver)
}
fn deliver_at(
    path: &Path,
    scope: &str,
    guid: &str,
    deliver: impl FnOnce() -> ApiResult<()>,
) -> ApiResult<()> {
    let mut db = open_ledger(path)?;
    let transaction = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "Notification storage busy.")?;
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM delivered WHERE scope=?1 AND guid=?2)",
            params![scope, guid],
            |row| row.get(0),
        )
        .map_err(|_| "Cannot read notification storage.")?;
    if exists {
        return Ok(());
    }
    deliver()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    transaction
        .execute(
            "INSERT INTO delivered VALUES (?1, ?2, ?3)",
            params![scope, guid, now],
        )
        .map_err(|_| "Cannot save notification delivery.")?;
    transaction
        .execute(
            "DELETE FROM delivered WHERE time < ?1",
            [now - 7 * 24 * 60 * 60],
        )
        .map_err(|_| "Cannot prune notification delivery.")?;
    transaction
        .commit()
        .map_err(|_| "Cannot save notification delivery.".into())
}

fn open_ledger(path: &Path) -> ApiResult<Connection> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let parent = path.parent().ok_or("Invalid notification storage path.")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create notification storage.")?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|_| "Cannot protect notification storage.")?;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .and_then(|file| file.set_permissions(fs::Permissions::from_mode(0o600)))
        .map_err(|_| "Cannot protect notification storage.")?;
    let db = Connection::open(path).map_err(|_| "Cannot open notification storage.")?;
    db.busy_timeout(Duration::from_secs(10))
        .map_err(|_| "Cannot initialize notification storage.")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS delivered (scope TEXT, guid TEXT, time INTEGER, PRIMARY KEY(scope,guid)); CREATE TABLE IF NOT EXISTS gui_lease (scope TEXT PRIMARY KEY, time INTEGER);")
        .map_err(|_| "Cannot initialize notification storage.")?;
    Ok(db)
}

fn update_lease(previous: Option<&str>, scope: Option<&str>) {
    let Ok(path) = ledger_path() else {
        return;
    };
    let Ok(db) = open_ledger(&path) else {
        return;
    };
    if previous != scope {
        if let Some(previous) = previous {
            let _ = db.execute("DELETE FROM gui_lease WHERE scope=?1", [previous]);
        }
    }
    if let Some(scope) = scope {
        let now = chrono::Utc::now().timestamp();
        let _ = db.execute(
            "INSERT OR REPLACE INTO gui_lease VALUES (?1, ?2)",
            params![scope, now],
        );
    }
}

/// Encrypted FCM envelopes have no message GUID. While a healthy GUI is polling
/// or streaming, it owns those alerts. The short lease expires after a crash.
pub fn gui_active(scope: &str) -> bool {
    let Ok(path) = ledger_path() else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    let Ok(db) = open_ledger(&path) else {
        return false;
    };
    db.query_row(
        "SELECT time FROM gui_lease WHERE scope=?1",
        [scope],
        |row| row.get::<_, i64>(0),
    )
    .is_ok_and(|time| (0..15).contains(&(chrono::Utc::now().timestamp() - time)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alerts_ignore_history_receipts_outgoing_and_replayed_messages() {
        let old = Message {
            guid: "old".into(),
            date_created: Some(10),
            ..Default::default()
        };
        let mut tracker = Tracker::default();
        tracker.reset(&[Chat {
            guid: "chat".into(),
            last_message: Some(old.clone()),
            display_name: None,
            participants: vec![],
        }]);
        assert!(!tracker.incoming("chat", &old));
        let mut new = Message {
            guid: "new".into(),
            date_created: Some(20),
            ..Default::default()
        };
        assert!(tracker.incoming("chat", &new));
        new.date_read = Some(30);
        assert!(!tracker.incoming("chat", &new));
        new.guid = "outgoing".into();
        new.is_from_me = true;
        assert!(!tracker.incoming("chat", &new));
        new.guid = "older".into();
        new.date_created = Some(5);
        new.is_from_me = false;
        assert!(!tracker.incoming("chat", &new));
        assert!(!tracker.incoming("new-chat", &new));
    }
    #[test]
    fn notification_delivery_is_shared_and_failed_delivery_can_retry() {
        let path = std::env::temp_dir()
            .join(format!("bb-alerts-{}", uuid::Uuid::new_v4()))
            .join("ledger.sqlite3");
        assert!(deliver_at(&path, "project", "message", || Err("offline".into())).is_err());
        deliver_at(&path, "project", "message", || Ok(())).unwrap();
        deliver_at(&path, "project", "message", || {
            panic!("duplicate across processes")
        })
        .unwrap();
        deliver_at(&path, "other-project", "message", || Ok(())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(body("Hi <b>&"), "Hi &lt;b&gt;&amp;");
    }
}
