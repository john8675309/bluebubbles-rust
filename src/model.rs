use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handle {
    #[serde(default)]
    pub address: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub guid: String,
    pub transfer_name: Option<String>,
    pub mime_type: Option<String>,
    pub total_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub guid: String,
    pub text: Option<String>,
    pub subject: Option<String>,
    #[serde(default)]
    pub is_from_me: bool,
    pub date_created: Option<i64>,
    pub date_read: Option<i64>,
    pub date_delivered: Option<i64>,
    pub error: Option<i64>,
    pub handle: Option<Handle>,
    #[serde(default, deserialize_with = "null_vec")]
    pub attachments: Vec<Attachment>,
    pub associated_message_guid: Option<String>,
    pub associated_message_type: Option<AssociatedMessageType>,
    pub thread_originator_guid: Option<String>,
    pub date_edited: Option<i64>,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The server serializes reaction names ("love", "like", "-love", etc.).
/// Retain numeric values too, including values written by earlier cache models.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum AssociatedMessageType {
    Name(String),
    Code(i64),
}

fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

impl Message {
    /// Match the server's activity ordering when creation time is unavailable.
    /// Read receipts must never promote an old conversation.
    pub fn activity_timestamp(&self) -> Option<i64> {
        self.date_created
            .filter(|time| *time > 0)
            .or_else(|| self.date_delivered.filter(|time| *time > 0))
    }
    pub fn preview(&self) -> String {
        self.text
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if self.attachments.is_empty() {
                    "Message".into()
                } else {
                    "Attachment".into()
                }
            })
    }

    pub fn delivery(&self) -> &'static str {
        if self.error.unwrap_or(0) != 0 {
            "Failed"
        } else if self.date_read.unwrap_or(0) > 0 {
            "Read"
        } else if self.date_delivered.unwrap_or(0) > 0 {
            "Delivered"
        } else {
            "Sent"
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub guid: String,
    pub display_name: Option<String>,
    #[serde(default, deserialize_with = "null_vec")]
    pub participants: Vec<Handle>,
    pub last_message: Option<Message>,
}

impl Chat {
    pub fn last_activity(&self) -> Option<i64> {
        self.last_message
            .as_ref()
            .and_then(Message::activity_timestamp)
    }
    pub fn title(&self) -> String {
        if let Some(name) = self.display_name.as_ref().filter(|s| !s.trim().is_empty()) {
            return name.clone();
        }
        let addresses: Vec<_> = self
            .participants
            .iter()
            .map(|h| h.address.as_str())
            .collect();
        if addresses.is_empty() {
            self.guid.clone()
        } else {
            addresses.join(", ")
        }
    }
}

pub fn merge_messages(current: &mut Vec<Message>, incoming: Vec<Message>) {
    for message in incoming {
        if let Some(old) = current.iter_mut().find(|m| m.guid == message.guid) {
            let mut incoming = message;
            incoming.date_read = incoming.date_read.max(old.date_read);
            incoming.date_delivered = incoming.date_delivered.max(old.date_delivered);
            if old.date_edited > incoming.date_edited {
                incoming.text = old.text.clone();
                incoming.date_edited = old.date_edited;
            }
            *old = incoming;
        } else {
            current.push(message);
        }
    }
    current.sort_by(|a, b| {
        a.date_created
            .cmp(&b.date_created)
            .then(a.guid.cmp(&b.guid))
    });
}
