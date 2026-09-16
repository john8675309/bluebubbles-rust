use crate::{
    api::ApiResult,
    model::{Chat, Message},
};
use rusqlite::{params, Connection};
use std::{collections::HashMap, fs::OpenOptions, path::Path, sync::mpsc, time::Duration};

#[derive(Default)]
pub struct Snapshot {
    pub chats: Vec<Chat>,
    pub messages: HashMap<String, Vec<Message>>,
    pub drafts: HashMap<String, String>,
}

pub struct OpenCache {
    pub cache: Cache,
    pub snapshot: Snapshot,
    pub errors: mpsc::Receiver<String>,
}

#[derive(Clone)]
pub struct Cache {
    tx: mpsc::Sender<Command>,
}

enum Command {
    Chats(Vec<Chat>),
    Messages(String, Vec<Message>),
    Draft(String, String),
    Flush(mpsc::Sender<()>),
}

impl Cache {
    pub fn open(server: &str) -> ApiResult<OpenCache> {
        let dirs = directories::ProjectDirs::from("app", "bluebubbles", "rust-linux")
            .ok_or("Cannot locate the application data directory.")?;
        Self::open_at(&dirs.data_local_dir().join("history.sqlite3"), server)
    }

    pub fn open_at(path: &Path, server: &str) -> ApiResult<OpenCache> {
        let parent = path.parent().ok_or("Invalid history path.")?;
        std::fs::create_dir_all(parent).map_err(|_| "Cannot create local history directory.")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot protect history directory.")?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(path)
            .map_err(|_| "Cannot open local history file.")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot protect history file.")?;
        }
        let mut conn = Connection::open(path).map_err(|_| "Cannot open the history database.")?;
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db_error)?;
        if version > 1 {
            return Err(
                "History was created by a newer client. Update this client to open it.".into(),
            );
        }
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS chats(server TEXT NOT NULL,guid TEXT NOT NULL,created INTEGER,payload TEXT NOT NULL,PRIMARY KEY(server,guid));
            CREATE TABLE IF NOT EXISTS messages(server TEXT NOT NULL,chat TEXT NOT NULL,guid TEXT NOT NULL,created INTEGER,payload TEXT NOT NULL,PRIMARY KEY(server,chat,guid));
            CREATE INDEX IF NOT EXISTS message_history ON messages(server,chat,created DESC);
            CREATE TABLE IF NOT EXISTS drafts(server TEXT NOT NULL,chat TEXT NOT NULL,text TEXT NOT NULL,PRIMARY KEY(server,chat));
            PRAGMA user_version=1;").map_err(db_error)?;
        let snapshot = load_snapshot(&conn, server)?;
        let server = server.to_string();
        let (tx, rx) = mpsc::channel();
        let (errors_tx, errors) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                if let Command::Flush(reply) = command {
                    let _ = reply.send(());
                    continue;
                }
                if save(&mut conn, &server, command).is_err() {
                    let _ = errors_tx.send("Could not save local history. Check available disk space and file permissions.".into());
                }
            }
        });
        Ok(OpenCache {
            cache: Self { tx },
            snapshot,
            errors,
        })
    }

    pub fn chats(&self, chats: Vec<Chat>) {
        let _ = self.tx.send(Command::Chats(chats));
    }
    pub fn messages(&self, chat: String, messages: Vec<Message>) {
        let _ = self.tx.send(Command::Messages(chat, messages));
    }
    pub fn draft(&self, chat: String, text: String) {
        let _ = self.tx.send(Command::Draft(chat, text));
    }
    pub fn flush(&self) -> ApiResult<()> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Command::Flush(tx))
            .map_err(|_| "History worker stopped.")?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "History flush timed out.".to_string())
    }
}

fn db_error(_: rusqlite::Error) -> String {
    "Could not read or write the history database.".into()
}

fn load_snapshot(conn: &Connection, server: &str) -> ApiResult<Snapshot> {
    let mut snapshot = Snapshot::default();
    let mut query = conn
        .prepare("SELECT payload FROM chats WHERE server=?1 ORDER BY created DESC LIMIT 100")
        .map_err(db_error)?;
    let rows = query
        .query_map([server], |row| row.get::<_, String>(0))
        .map_err(db_error)?;
    for row in rows {
        snapshot.chats.push(
            serde_json::from_str(&row.map_err(db_error)?).map_err(|_| "Invalid cached chat.")?,
        );
    }
    let mut query = conn.prepare("SELECT payload FROM messages WHERE server=?1 AND chat=?2 ORDER BY created DESC LIMIT 100").map_err(db_error)?;
    for chat in &snapshot.chats {
        let mut messages = Vec::new();
        for row in query
            .query_map(params![server, chat.guid], |row| row.get::<_, String>(0))
            .map_err(db_error)?
        {
            messages.push(
                serde_json::from_str(&row.map_err(db_error)?)
                    .map_err(|_| "Invalid cached message.")?,
            );
        }
        messages.reverse();
        snapshot.messages.insert(chat.guid.clone(), messages);
    }
    let mut query = conn
        .prepare("SELECT chat,text FROM drafts WHERE server=?1")
        .map_err(db_error)?;
    for row in query
        .query_map([server], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?
    {
        let (chat, text) = row.map_err(db_error)?;
        snapshot.drafts.insert(chat, text);
    }
    Ok(snapshot)
}

fn save(conn: &mut Connection, server: &str, command: Command) -> ApiResult<()> {
    let tx = conn.transaction().map_err(db_error)?;
    match command {
        Command::Chats(chats) => {
            for chat in chats {
                let payload = serde_json::to_string(&chat).map_err(|_| "Cannot encode chat.")?;
                tx.execute("INSERT INTO chats VALUES(?1,?2,?3,?4) ON CONFLICT(server,guid) DO UPDATE SET created=excluded.created,payload=excluded.payload",params![server,chat.guid,chat.last_activity(),payload]).map_err(db_error)?;
            }
        }
        Command::Messages(chat, messages) => {
            for message in messages {
                let payload =
                    serde_json::to_string(&message).map_err(|_| "Cannot encode message.")?;
                tx.execute("INSERT INTO messages VALUES(?1,?2,?3,?4,?5) ON CONFLICT(server,chat,guid) DO UPDATE SET created=excluded.created,payload=excluded.payload",params![server,chat,message.guid,message.date_created,payload]).map_err(db_error)?;
            }
        }
        Command::Draft(chat, text) => {
            if text.is_empty() {
                tx.execute(
                    "DELETE FROM drafts WHERE server=?1 AND chat=?2",
                    params![server, chat],
                )
                .map_err(db_error)?;
            } else {
                tx.execute("INSERT INTO drafts VALUES(?1,?2,?3) ON CONFLICT(server,chat) DO UPDATE SET text=excluded.text",params![server,chat,text]).map_err(db_error)?;
            }
        }
        Command::Flush(_) => {}
    }
    tx.commit().map_err(db_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_history_and_drafts_and_isolates_servers() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("history.sqlite3");
        let first = Cache::open_at(&path, "https://one/").unwrap();
        let message = Message {
            guid: "message".into(),
            text: Some("hello".into()),
            date_created: Some(10),
            associated_message_type: Some(crate::model::AssociatedMessageType::Name("love".into())),
            ..Default::default()
        };
        first.cache.chats(vec![Chat {
            guid: "chat".into(),
            display_name: Some("Name".into()),
            participants: vec![],
            last_message: Some(message.clone()),
        }]);
        first.cache.messages("chat".into(), vec![message]);
        first.cache.draft("chat".into(), "draft".into());
        first.cache.flush().unwrap();
        let restored = Cache::open_at(&path, "https://one/").unwrap();
        assert_eq!(
            restored.snapshot.messages["chat"][0].text.as_deref(),
            Some("hello")
        );
        assert_eq!(restored.snapshot.drafts["chat"], "draft");
        assert_eq!(
            restored.snapshot.messages["chat"][0].associated_message_type,
            Some(crate::model::AssociatedMessageType::Name("love".into()))
        );
        assert!(Cache::open_at(&path, "https://two/")
            .unwrap()
            .snapshot
            .chats
            .is_empty());
        restored.cache.draft("chat".into(), String::new());
        restored.cache.flush().unwrap();
        assert!(Cache::open_at(&path, "https://one/")
            .unwrap()
            .snapshot
            .drafts
            .is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(first);
        drop(restored);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
