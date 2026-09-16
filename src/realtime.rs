use crate::api::Api;
use rust_socketio::{ClientBuilder, Payload};
use serde_json::Value;
use std::{
    sync::{mpsc, Arc},
    time::Duration,
};

#[derive(Clone, Debug)]
pub enum LiveEvent {
    Connected,
    Disconnected,
    Data { name: String, data: Value },
}

pub struct LiveConnection {
    stop: mpsc::Sender<()>,
}

impl LiveConnection {
    pub fn start(api: Api, callback: impl Fn(LiveEvent) + Send + Sync + 'static) -> Self {
        let (stop, receiver) = mpsc::channel();
        let callback = Arc::new(callback);
        std::thread::spawn(move || {
            let mut delay = 1;
            loop {
                let opened = callback.clone();
                let closed = callback.clone();
                let failed = callback.clone();
                let mut builder = ClientBuilder::new(api.socket_url())
                    .reconnect(true)
                    .reconnect_on_disconnect(true)
                    .reconnect_delay(1000, 30000)
                    .on("open", move |_, _| opened(LiveEvent::Connected))
                    .on("close", move |_, _| closed(LiveEvent::Disconnected))
                    .on("error", move |_, _| failed(LiveEvent::Disconnected));
                for name in [
                    "new-message",
                    "updated-message",
                    "group-name-change",
                    "participant-added",
                    "participant-removed",
                    "participant-left",
                    "chat-read-status-changed",
                    "typing-indicator",
                    "incoming-facetime",
                    "ft-call-status-changed",
                    "new-findmy-location",
                    "imessage-aliases-removed",
                ] {
                    let cb = callback.clone();
                    builder = builder.on(name, move |payload, _| {
                        if let Some(data) = decode_payload(payload) {
                            cb(LiveEvent::Data {
                                name: name.to_string(),
                                data,
                            });
                        }
                    });
                }
                match builder.connect() {
                    Ok(client) => {
                        // Socket.IO owns reconnection after the initial handshake.
                        let _ = receiver.recv();
                        let _ = client.disconnect();
                        break;
                    }
                    Err(_) => {
                        // Library errors can contain the password-bearing URL.
                        callback(LiveEvent::Disconnected);
                        if !matches!(
                            receiver.recv_timeout(Duration::from_secs(delay)),
                            Err(mpsc::RecvTimeoutError::Timeout)
                        ) {
                            break;
                        }
                        delay = (delay * 2).min(30);
                    }
                }
            }
        });
        Self { stop }
    }
}

impl Drop for LiveConnection {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

#[allow(deprecated)]
fn decode_payload(payload: Payload) -> Option<Value> {
    let value = match payload {
        Payload::Text(values) => values.into_iter().next()?,
        Payload::String(text) => Value::String(text),
        Payload::Binary(_) => return None,
    };
    let value = match value {
        Value::String(text) => serde_json::from_str(&text).ok()?,
        value => value,
    };
    if value.get("encrypted").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if value.get("guid").is_some() || value.get("chatGuid").is_some() {
        Some(value)
    } else {
        Some(value.get("data").cloned().unwrap_or(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_raw_wrapped_and_json_string_payloads() {
        let message = json!({"guid":"m1", "text":"hello"});
        for payload in [
            Payload::from(message.clone()),
            Payload::from(json!({"data":message.clone()})),
            Payload::Text(vec![Value::String(message.to_string())]),
        ] {
            assert_eq!(decode_payload(payload), Some(message.clone()));
        }
    }

    #[test]
    fn encrypted_and_binary_payloads_are_not_misinterpreted() {
        assert!(
            decode_payload(Payload::from(json!({"encrypted":true,"data":"ciphertext"}))).is_none()
        );
        assert!(decode_payload(Payload::from(vec![0u8, 1, 2])).is_none());
    }
}
