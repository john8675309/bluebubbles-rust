//! Optional FCM receiver, independent of the GUI process.
use crate::{
    api::{Api, ApiResult},
    firebase::FirebaseConfig,
};
use fcm_push_listener::{Message, MessageStream, Registration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

#[derive(prost::Message)]
struct LoginResponse {
    #[prost(bytes = "vec", optional, tag = "3")]
    error: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct SavedRegistration {
    project: String,
    registration: Registration,
    persistent_ids: Vec<String>,
    message_ids: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Preferences {
    pub previews: bool,
}

fn directory() -> ApiResult<PathBuf> {
    directories::ProjectDirs::from("app", "bluebubbles", "rust-linux")
        .map(|d| d.data_local_dir().join("push"))
        .ok_or("Cannot locate notification storage.".into())
}

pub fn state_path(config: &FirebaseConfig) -> ApiResult<PathBuf> {
    config.endpoint()?;
    Ok(directory()?.join(format!("{}.json", config.project_id)))
}

fn private_file(path: &Path) -> ApiResult<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let parent = path.parent().ok_or("Invalid notification storage path.")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create notification storage.")?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|_| "Cannot protect notification storage.")?;
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|_| "Cannot open notification storage.")?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| "Cannot protect notification credentials.")?;
    Ok(file)
}

fn save_private(path: &Path, bytes: &[u8]) -> ApiResult<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = private_file(&temporary)?;
    let result = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .and_then(|_| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|_| "Cannot save notification state.".into())
}

fn save(path: &Path, state: &SavedRegistration) -> ApiResult<()> {
    save_private(
        path,
        &serde_json::to_vec(state).map_err(|_| "Cannot encode notification state.")?,
    )
}

pub fn enabled(config: &FirebaseConfig) -> bool {
    state_path(config).is_ok_and(|path| path.with_extension("enabled").exists())
}

pub fn preferences(config: &FirebaseConfig) -> (Preferences, bool) {
    let Ok(path) = state_path(config) else {
        return (Preferences::default(), false);
    };
    let prefs = fs::read(path.with_extension("enabled"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let at_login = desktop_path(&path).is_ok_and(|path| path.exists());
    (prefs, at_login)
}

pub fn status(config: &FirebaseConfig) -> String {
    let Ok(path) = state_path(config) else {
        return "Not configured".into();
    };
    if !path.with_extension("enabled").exists() {
        return "Disabled".into();
    }
    fs::read_to_string(path.with_extension("status"))
        .unwrap_or_else(|_| "Starting background receiver…".into())
}

fn write_status(path: &Path, message: &str) {
    let _ = save_private(&path.with_extension("status"), message.as_bytes());
}

fn spawn_worker(path: &Path) -> ApiResult<()> {
    let executable =
        std::env::current_exe().map_err(|_| "Cannot locate the notification executable.")?;
    let mut child = Command::new(executable)
        .arg("--push-worker")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start background notifications.")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn resume_workers() {
    let Ok(dir) = directory() else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|e| e == "enabled") {
            let _ = spawn_worker(&entry.path().with_extension("json"));
        }
    }
}

pub fn enable(
    api: Api,
    expected: FirebaseConfig,
    preferences: Preferences,
    at_login: bool,
) -> ApiResult<()> {
    let path = state_path(&expected)?;
    // Serialize registration attempts across windows and reuse existing tokens.
    let lock = private_file(&path.with_extension("setup-lock"))?;
    lock.try_lock()
        .map_err(|_| "Notification setup is already running.")?;
    let source: Value = api.json(api.client.get(api.endpoint(&["fcm", "client"])))?;
    let config = FirebaseConfig::from_google_services(&source)?;
    if config != expected {
        return Err(
            "Firebase configuration changed. Load it again before enabling notifications.".into(),
        );
    }
    let state = if path.exists() {
        serde_json::from_slice::<SavedRegistration>(
            &fs::read(&path).map_err(|_| "Cannot read notification credentials.")?,
        )
        .map_err(|_| "Cannot read saved notification registration.".to_string())?
    } else {
        let app = source
            .pointer("/client/0/client_info/mobilesdk_app_id")
            .and_then(Value::as_str)
            .ok_or("Firebase client configuration has no Android app ID.")?;
        let key = source
            .pointer("/client/0/api_key/0/current_key")
            .and_then(Value::as_str)
            .ok_or("Firebase client configuration has no API key.")?;
        crate::initialize_tls();
        let runtime =
            tokio::runtime::Runtime::new().map_err(|_| "Cannot start FCM registration.")?;
        let registration = runtime.block_on(async {
            let http = fcm_http::Client::builder().timeout(Duration::from_secs(30)).redirect(fcm_http::redirect::Policy::none()).build().map_err(|_| "Cannot initialize FCM registration.")?;
            tokio::time::timeout(Duration::from_secs(90), fcm_push_listener::register(&http, app, &config.project_id, key, None)).await
                .map_err(|_| "FCM registration timed out.")?.map_err(|_| "FCM registration failed. Check the Firebase client configuration and network access.")
        })?;
        SavedRegistration {
            project: config.project_id.clone(),
            registration,
            persistent_ids: vec![],
            message_ids: vec![],
        }
    };
    if state.project != expected.project_id {
        return Err("Notification registration belongs to another project.".into());
    }
    // Save before registering with the Mac so explicit retries reuse the token.
    if !path.exists() {
        save(&path, &state)?;
    }
    api.json::<Value>(api.client.post(api.endpoint(&["fcm", "device"])).json(
        &serde_json::json!({
            "name":"BlueBubbles Rust Linux", "identifier":state.registration.fcm_token
        }),
    ))?;
    save_private(
        &path.with_extension("enabled"),
        &serde_json::to_vec(&preferences).map_err(|_| "Cannot save notification preferences.")?,
    )?;
    set_autostart(&path, at_login)?;
    spawn_worker(&path)
}

fn desktop_path(path: &Path) -> ApiResult<PathBuf> {
    let base = directories::BaseDirs::new().ok_or("Cannot locate startup settings.")?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid notification project.")?;
    Ok(base
        .config_dir()
        .join("autostart")
        .join(format!("bluebubbles-push-{name}.desktop")))
}

fn desktop_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
            .replace('%', "%%")
    )
}

fn set_autostart(path: &Path, enabled: bool) -> ApiResult<()> {
    let desktop = desktop_path(path)?;
    if !enabled {
        if desktop.exists() {
            fs::remove_file(desktop).map_err(|_| "Cannot remove notification startup entry.")?;
        }
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|_| "Cannot locate notification executable.")?;
    if exe.to_string_lossy().contains(['\n', '\r']) || path.to_string_lossy().contains(['\n', '\r'])
    {
        return Err("Unsupported executable path for startup entry.".into());
    }
    let contents = format!("[Desktop Entry]\nType=Application\nName=BlueBubbles notifications\nExec={} --push-worker {}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", desktop_quote(&exe.to_string_lossy()), desktop_quote(&path.to_string_lossy()));
    if contents.contains('\r') {
        return Err("Unsupported executable path for startup entry.".into());
    }
    fs::create_dir_all(desktop.parent().unwrap())
        .map_err(|_| "Cannot create startup directory.")?;
    fs::write(desktop, contents).map_err(|_| "Cannot save notification startup entry.".into())
}

pub fn disable(config: &FirebaseConfig) -> ApiResult<()> {
    let path = state_path(config)?;
    let marker = path.with_extension("enabled");
    if marker.exists() {
        fs::remove_file(marker).map_err(|_| "Cannot disable notifications.")?;
    }
    set_autostart(&path, false)
}

pub fn reset(config: &FirebaseConfig) -> ApiResult<()> {
    disable(config)?;
    let path = state_path(config)?;
    let lock = private_file(&path.with_extension("lock"))?;
    for _ in 0..30 {
        if lock.try_lock().is_ok() {
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|_| "Cannot remove the saved notification registration.")?;
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("The receiver is still stopping. Try resetting registration again shortly.".into())
}

pub fn test_notification() -> ApiResult<()> {
    notify_rust::Notification::new()
        .appname("BlueBubbles")
        .summary("BlueBubbles")
        .body("Desktop notifications are working.")
        .icon("app.bluebubbles.RustLinux")
        .show()
        .map(|_| ())
        .map_err(|_| {
            "Desktop notifications are unavailable. Check your desktop notification service.".into()
        })
}

/// Keeps running after the GUI exits. Stop requests are checked every two seconds.
pub fn run_worker(path: &Path) -> ApiResult<()> {
    let result = worker_loop(path);
    if let Err(error) = &result {
        write_status(path, error);
    }
    result
}

fn worker_loop(path: &Path) -> ApiResult<()> {
    crate::initialize_tls();
    let lock = private_file(&path.with_extension("lock"))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    if !path.with_extension("enabled").exists() {
        return Ok(());
    }
    let mut state: SavedRegistration =
        serde_json::from_slice(&fs::read(path).map_err(|_| "Cannot read FCM registration.")?)
            .map_err(|_| "Invalid FCM registration.")?;
    let runtime =
        tokio::runtime::Runtime::new().map_err(|_| "Cannot start notification receiver.")?;
    runtime.block_on(async {
        let mut delay = 2;
        loop {
            if !path.with_extension("enabled").exists() {
                break;
            }
            write_status(path, "Connecting to Firebase Cloud Messaging…");
            let receiver = receive(path, &mut state);
            tokio::pin!(receiver);
            let result = loop {
                tokio::select! {
                    result = &mut receiver => break result,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {
                        if !path.with_extension("enabled").exists() { return Ok(()); }
                    }
                }
            };
            if let Err(error) = result {
                write_status(path, &error);
            }
            for _ in 0..delay {
                if !path.with_extension("enabled").exists() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            delay = (delay * 2).min(60);
        }
        Ok(())
    })
}

async fn receive(path: &Path, state: &mut SavedRegistration) -> ApiResult<()> {
    let http = fcm_http::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Cannot initialize FCM connection.")?;
    let session = state
        .registration
        .gcm
        .checkin(&http)
        .await
        .map_err(|_| "FCM check-in failed; retrying…")?;
    if session.changed(&state.registration.gcm) {
        return Err(
            "FCM device credentials changed. Disable and reset registration before retrying."
                .into(),
        );
    }
    let connection = tokio::time::timeout(
        Duration::from_secs(45),
        session.new_connection(state.persistent_ids.clone()),
    )
    .await
    .map_err(|_| "FCM connection timed out; retrying…")?
    .map_err(|_| "FCM connection failed; retrying…")?;
    let mut stream = MessageStream::wrap(connection, &state.registration.keys);
    while let Some(message) = tokio::time::timeout(Duration::from_secs(35 * 60), stream.next())
        .await
        .map_err(|_| "FCM heartbeat timed out; reconnecting…")?
    {
        match message.map_err(|_| "FCM stream interrupted; reconnecting…")? {
            Message::HeartbeatPing => {
                stream
                    .write_all(&fcm_push_listener::new_heartbeat_ack())
                    .await
                    .map_err(|_| "FCM heartbeat failed; reconnecting…")?;
            }
            Message::Other(3, bytes) => {
                use prost::Message;
                let login =
                    LoginResponse::decode(bytes).map_err(|_| "Invalid FCM login response.")?;
                if login.error.is_some() {
                    return Err(
                        "FCM rejected the registration. Reset registration in Firebase settings."
                            .into(),
                    );
                }
                write_status(path, "Receiving notifications in the background");
            }
            Message::Other(4 | 10, _) => return Err("FCM closed the session; reconnecting…".into()),
            Message::Data(data) => {
                let scope = format!("firebase:{}", state.project);
                process_data(path, state, data, |body, id| async move {
                    tokio::task::spawn_blocking(move || {
                        let deliver = || {
                            notify_rust::Notification::new()
                                .appname("BlueBubbles")
                                .summary("BlueBubbles")
                                .body(&body)
                                .icon("app.bluebubbles.RustLinux")
                                .show()
                                .map(|_| ())
                                .map_err(|_| "Desktop notifications unavailable; retrying…".into())
                        };
                        if let Some(id) = id {
                            crate::notifications::deliver_once(&scope, &id, deliver)
                        } else if body == "New message" && crate::notifications::gui_active(&scope)
                        {
                            Ok(())
                        } else {
                            deliver()
                        }
                    })
                    .await
                    .map_err(|_| "Desktop notification worker failed.")?
                    .map_err(|_| "Desktop notifications unavailable; retrying…".into())
                })
                .await?;
            }
            _ => {}
        }
    }
    Err("FCM disconnected; reconnecting…".into())
}

async fn process_data<F, Fut>(
    path: &Path,
    state: &mut SavedRegistration,
    data: fcm_push_listener::DataMessage,
    deliver: F,
) -> ApiResult<()>
where
    F: FnOnce(String, Option<String>) -> Fut,
    Fut: std::future::Future<Output = ApiResult<()>>,
{
    if data
        .persistent_id
        .as_ref()
        .is_some_and(|id| state.persistent_ids.contains(id))
    {
        return Ok(());
    }
    let preferences: Preferences = fs::read(path.with_extension("enabled"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if let Some((id, body)) = notification(&data.body, preferences.previews) {
        if id.as_ref().is_none_or(|id| !state.message_ids.contains(id)) {
            deliver(body, id.clone()).await?;
            if let Some(id) = id {
                state.message_ids.push(id);
            }
        }
    }
    if let Some(id) = data.persistent_id {
        state.persistent_ids.push(id);
    }
    trim_ids(&mut state.persistent_ids, 2048);
    trim_ids(&mut state.message_ids, 2048);
    save(path, state)
}

fn trim_ids(ids: &mut Vec<String>, max: usize) {
    if ids.len() > max {
        ids.drain(..ids.len() - max);
    }
}

fn notification(bytes: &[u8], previews: bool) -> Option<(Option<String>, String)> {
    let payload: Value = serde_json::from_slice(bytes).ok()?;
    let data = payload.get("data")?;
    let kind = data.get("type")?.as_str()?;
    if kind != "new-message" && kind != "incoming-facetime" {
        return None;
    }
    let raw = data.get("data")?;
    let raw: Value = match raw {
        Value::String(text) => serde_json::from_str(text).ok()?,
        other => other.clone(),
    };
    let message = raw.get("data").unwrap_or(&raw);
    if message.get("isFromMe").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let id = message
        .get("guid")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let body = if kind == "incoming-facetime" {
        "Incoming FaceTime call".into()
    } else if previews && raw.get("encrypted").and_then(Value::as_bool) != Some(true) {
        message
            .get("text")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("New message")
            .chars()
            .take(240)
            .collect::<String>()
    } else {
        "New message".into()
    };
    // Desktop notification bodies accept markup; treat message text literally.
    Some((
        id,
        body.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_delivery_deduplicates_after_restart_and_retries_failed_notifications() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("state.json");
        let mut state: SavedRegistration = serde_json::from_slice(&serde_json::to_vec(&json!({
            "project":"bb-test", "registration":{"fcm_token":"test-token","gcm":{"android_id":"1","security_token":"2"},
                "keys":{"public_key":"","private_key":"","auth_secret":""}}, "persistent_ids":[], "message_ids":[]
        })).unwrap()).unwrap();
        let data = |id: &str| {
            fcm_push_listener::DataMessage {persistent_id:Some(id.into()),body:serde_json::to_vec(&json!({"data":{"type":"new-message","data":"{\"guid\":\"message1\",\"text\":\"secret message body\"}"}})).unwrap()}
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            assert!(process_data(&path, &mut state, data("one"), |_, _| async {
                Err("No desktop bus".into())
            })
            .await
            .is_err());
            assert!(state.persistent_ids.is_empty());
            process_data(&path, &mut state, data("one"), |body, _| async move {
                assert_eq!(body, "New message");
                Ok(())
            })
            .await
            .unwrap();
            let mut restored: SavedRegistration =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            for id in ["one", "two"] {
                process_data(&path, &mut restored, data(id), |_, _| async {
                    panic!("Duplicate notification")
                })
                .await
                .unwrap();
            }
            assert_eq!(restored.persistent_ids, vec!["one", "two"]);
            assert_eq!(restored.message_ids, vec!["message1"]);
            assert!(!String::from_utf8(fs::read(&path).unwrap())
                .unwrap()
                .contains("secret message body"));
        });
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn notifications_filter_receipts_outgoing_and_protect_previews() {
        let make = |kind: &str, message: Value| {
            serde_json::to_vec(&json!({"data":{"type":kind,"data":message.to_string()}})).unwrap()
        };
        let bytes = make(
            "new-message",
            json!({"guid":"m","text":"<private> & secret","isFromMe":false}),
        );
        assert_eq!(
            notification(&bytes, false).unwrap(),
            (Some("m".into()), "New message".into())
        );
        assert_eq!(
            notification(&bytes, true).unwrap().1,
            "&lt;private&gt; &amp; secret"
        );
        assert!(notification(&make("updated-message", json!({"guid":"m"})), true).is_none());
        assert!(notification(
            &make("new-message", json!({"guid":"m","isFromMe":true})),
            true
        )
        .is_none());
        assert_eq!(
            notification(
                &make("new-message", json!({"encrypted":true,"data":"ciphertext"})),
                true
            )
            .unwrap()
            .1,
            "New message"
        );
    }

    #[test]
    fn notification_storage_is_private_and_atomic() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("state.json");
        save_private(&path, b"first").unwrap();
        save_private(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let first = private_file(&dir.join("lock")).unwrap();
        first.try_lock().unwrap();
        assert!(private_file(&dir.join("lock")).unwrap().try_lock().is_err());
        drop(first);
        fs::remove_dir_all(dir).unwrap();
    }
}
