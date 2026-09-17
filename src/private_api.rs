//! Capability checks and per-conversation Private API composer state.
use bluebubbles_linux::{
    api_actions::{Action, ServerInfo},
    model::Message,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub subject: String,
    pub effect: String,
    pub reply: Option<(String, String)>,
}
impl Extras {
    pub fn is_empty(&self) -> bool {
        self.subject.is_empty() && self.effect.is_empty() && self.reply.is_none()
    }
}

#[derive(Clone)]
pub enum Dialog {
    Edit {
        chat: String,
        message: Box<Message>,
        text: String,
    },
    Confirm {
        action: Action,
        description: String,
    },
    Group {
        chat: String,
        name: String,
        address: String,
    },
}

pub fn imessage(chat: &str) -> bool {
    chat.starts_with("iMessage;")
}
pub fn group(chat: &str) -> bool {
    chat.starts_with("iMessage;+;")
}
pub fn editable(
    info: Option<&ServerInfo>,
    chat: &str,
    message: &Message,
    unsend: bool,
    now: i64,
) -> bool {
    info.is_some_and(|info| {
        info.private_available() && info.macos_at_least(13) && info.server_at_least(1, 2, 6)
    }) && imessage(chat)
        && message.is_from_me
        && message.associated_message_guid.is_none()
        && message.error.unwrap_or(0) == 0
        && !message.is_unsent()
        && message
            .date_created
            .is_some_and(|time| (0..if unsend { 120_000 } else { 900_000 }).contains(&(now - time)))
        && (unsend || message.text.as_ref().is_some_and(|text| !text.is_empty()))
}

pub const EFFECTS: &[(&str, &str)] = &[
    ("None", ""),
    ("Slam", "com.apple.MobileSMS.expressivesend.impact"),
    ("Loud", "com.apple.MobileSMS.expressivesend.loud"),
    ("Gentle", "com.apple.MobileSMS.expressivesend.gentle"),
    (
        "Invisible ink",
        "com.apple.MobileSMS.expressivesend.invisibleink",
    ),
    ("Echo", "com.apple.messages.effect.CKEchoEffect"),
    ("Spotlight", "com.apple.messages.effect.CKSpotlightEffect"),
    (
        "Balloons",
        "com.apple.messages.effect.CKHappyBirthdayEffect",
    ),
    ("Confetti", "com.apple.messages.effect.CKConfettiEffect"),
    ("Love", "com.apple.messages.effect.CKHeartEffect"),
    ("Lasers", "com.apple.messages.effect.CKLasersEffect"),
    ("Fireworks", "com.apple.messages.effect.CKFireworksEffect"),
    ("Celebration", "com.apple.messages.effect.CKSparklesEffect"),
];
pub const REACTIONS: &[(&str, &str)] = &[
    ("Love", "love"),
    ("Like", "like"),
    ("Dislike", "dislike"),
    ("Laugh", "laugh"),
    ("Emphasize", "emphasize"),
    ("Question", "question"),
];

pub struct TypingWorker(std::sync::mpsc::Sender<Option<String>>);
impl TypingWorker {
    pub fn start(api: bluebubbles_linux::api::Api) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        std::thread::spawn(move || {
            use std::{
                sync::mpsc::RecvTimeoutError,
                time::{Duration, Instant},
            };
            let mut current: Option<String> = None;
            let mut last_sent = Instant::now() - Duration::from_secs(5);
            loop {
                let result = rx.recv_timeout(Duration::from_secs(5));
                let disconnected = matches!(result, Err(RecvTimeoutError::Disconnected));
                let next = result.unwrap_or(None);
                if next != current {
                    if let Some(chat) = current.take() {
                        let _ = api.perform(Action::Typing {
                            chat,
                            active: false,
                        });
                    }
                    last_sent = Instant::now() - Duration::from_secs(5);
                }
                if let Some(chat) = &next {
                    if last_sent.elapsed() >= Duration::from_secs(4) {
                        let _ = api.perform(Action::Typing {
                            chat: chat.clone(),
                            active: true,
                        });
                        last_sent = Instant::now();
                    }
                }
                current = next;
                if disconnected {
                    break;
                }
            }
        });
        Self(tx)
    }
    pub fn update(&self, chat: Option<String>) {
        let _ = self.0.send(chat);
    }
}

impl crate::app::App {
    pub fn private_available(&self) -> bool {
        self.private_enabled
            && self
                .server_info
                .as_ref()
                .is_some_and(|info| info.private_available() && info.server_at_least(0, 4, 0))
    }
    pub fn private_for(&self, chat: &str) -> bool {
        self.private_available() && imessage(chat)
    }
    pub fn refresh_capabilities(&mut self, ctx: &eframe::egui::Context) {
        if let Some(api) = self.api.clone() {
            self.run(ctx, "Checking Private API…", move || {
                api.info().map(crate::app::Event::ServerInfo)
            });
        }
    }
    pub fn typing_changed(&mut self, chat: &str) {
        if self.send_typing && self.private_for(chat) {
            if self.typing_worker.is_none() {
                if let Some(api) = self.api.clone() {
                    self.typing_worker = Some(TypingWorker::start(api));
                }
            }
            if let Some(worker) = &self.typing_worker {
                worker.update(
                    self.drafts
                        .get(chat)
                        .filter(|text| !text.is_empty())
                        .map(|_| chat.to_owned()),
                );
            }
        } else {
            self.stop_typing();
        }
    }
    pub fn stop_typing(&mut self) {
        if let Some(worker) = &self.typing_worker {
            worker.update(None);
        }
    }
    pub fn private_action(&mut self, ctx: &eframe::egui::Context, action: Action) {
        if self.busy {
            return;
        }
        if !self.private_available() {
            self.error = Some("Private API is unavailable. Enable it on your Mac and check that its helper is connected.".into());
            return;
        }
        let allowed = match &action {
            Action::Edit { message, .. } | Action::Unsend { message } => {
                self.messages.iter().any(|(chat, messages)| {
                    messages.iter().any(|m| {
                        &m.guid == message
                            && editable(
                                self.server_info.as_ref(),
                                chat,
                                m,
                                matches!(action, Action::Unsend { .. }),
                                chrono::Utc::now().timestamp_millis(),
                            )
                    })
                })
            }
            Action::Read { read: false, .. } => self
                .server_info
                .as_ref()
                .is_some_and(|info| info.macos_at_least(13)),
            Action::Rename { chat, .. }
            | Action::Participant { chat, .. }
            | Action::Leave { chat } => group(chat),
            Action::React { chat, .. } => imessage(chat),
            _ => true,
        };
        if !allowed {
            self.error = Some(
                "This action is not supported for this message, conversation, or Mac version."
                    .into(),
            );
            return;
        }
        if let Some(api) = self.api.clone() {
            self.run(ctx, "Applying message action…", move || {
                let info = api.info()?;
                if !info.private_available() { return Err("The Private API helper is disconnected. Reconnect it on your Mac and retry.".into()); }
                let value = api.perform(action.clone()).map_err(|error| format!("{error} Check the conversation before retrying; the server may have applied the action."))?;
                Ok(crate::app::Event::PrivateComplete { action, value })
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_and_unsend_require_capabilities_ownership_and_time_window() {
        let info = ServerInfo {
            private_api: true,
            helper_connected: true,
            os_version: "13.0".into(),
            server_version: "1.2.6".into(),
        };
        let mut message = Message {
            is_from_me: true,
            text: Some("hello".into()),
            date_created: Some(1_000),
            ..Default::default()
        };
        assert!(editable(
            Some(&info),
            "iMessage;-;a",
            &message,
            false,
            121_000
        ));
        assert!(!editable(
            Some(&info),
            "iMessage;-;a",
            &message,
            true,
            121_000
        ));
        assert!(!editable(
            Some(&info),
            "iMessage;-;a",
            &message,
            false,
            901_000
        ));
        assert!(!editable(Some(&info), "SMS;-;a", &message, false, 2_000));
        assert!(!editable(None, "iMessage;-;a", &message, false, 2_000));
        message.is_from_me = false;
        assert!(!editable(
            Some(&info),
            "iMessage;-;a",
            &message,
            false,
            2_000
        ));
    }
}
