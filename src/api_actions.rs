use crate::{
    api::{Api, ApiResult},
    model::Message,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServerInfo {
    #[serde(default, alias = "serverVersion")]
    pub server_version: String,
    #[serde(default, alias = "osVersion")]
    pub os_version: String,
    #[serde(default)]
    pub private_api: bool,
    #[serde(default)]
    pub helper_connected: bool,
}

impl ServerInfo {
    pub fn private_available(&self) -> bool {
        self.private_api && self.helper_connected
    }
    pub fn macos_at_least(&self, version: u32) -> bool {
        self.os_version
            .split('.')
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|v| v >= version)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SendOptions {
    pub private_api: bool,
    pub subject: Option<String>,
    pub effect: Option<String>,
    pub reply: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Action {
    Read {
        chat: String,
        read: bool,
    },
    Typing {
        chat: String,
        active: bool,
    },
    Rename {
        chat: String,
        name: String,
    },
    Participant {
        chat: String,
        address: String,
        add: bool,
    },
    Leave {
        chat: String,
    },
    DeleteChat {
        chat: String,
    },
    DeleteMessage {
        chat: String,
        message: String,
    },
    React {
        chat: String,
        message: String,
        text: String,
        reaction: String,
    },
    Edit {
        message: String,
        text: String,
        original: String,
    },
    Unsend {
        message: String,
    },
    Schedule {
        chat: String,
        message: String,
        when: i64,
        private_api: bool,
    },
    DeleteSchedule {
        id: i64,
    },
}

impl Api {
    pub fn info(&self) -> ApiResult<ServerInfo> {
        self.json(self.client.get(self.endpoint(&["server", "info"])))
    }
    pub fn contacts(&self) -> ApiResult<Vec<Value>> {
        self.json(self.client.get(self.endpoint(&["contact"])))
    }
    pub fn schedules(&self) -> ApiResult<Vec<Value>> {
        self.json(self.client.get(self.endpoint(&["message", "schedule"])))
    }

    pub fn send_with_options(
        &self,
        chat: &str,
        text: &str,
        temp_guid: &str,
        options: SendOptions,
    ) -> ApiResult<()> {
        let mut body = json!({"chatGuid":chat,"tempGuid":temp_guid,"message":text,"method":if options.private_api {"private-api"} else {"apple-script"}});
        if options.private_api {
            body["subject"] = json!(options.subject);
            body["effectId"] = json!(options.effect);
            body["selectedMessageGuid"] = json!(options.reply);
            body["partIndex"] = json!(0);
        }
        self.json::<Value>(
            self.client
                .post(self.endpoint(&["message", "text"]))
                .json(&body),
        )?;
        Ok(())
    }

    pub fn perform(&self, action: Action) -> ApiResult<Value> {
        let (method, path, body) = match action {
            Action::Read { chat, read } => (
                Method::POST,
                vec![
                    "chat".into(),
                    chat,
                    if read { "read" } else { "unread" }.into(),
                ],
                json!({}),
            ),
            Action::Typing { chat, active } => (
                if active { Method::POST } else { Method::DELETE },
                vec!["chat".into(), chat, "typing".into()],
                json!({}),
            ),
            Action::Rename { chat, name } => (
                Method::PUT,
                vec!["chat".into(), chat],
                json!({"displayName":name}),
            ),
            Action::Participant { chat, address, add } => (
                Method::POST,
                vec![
                    "chat".into(),
                    chat,
                    "participant".into(),
                    if add { "add" } else { "remove" }.into(),
                ],
                json!({"address":address}),
            ),
            Action::Leave { chat } => (
                Method::POST,
                vec!["chat".into(), chat, "leave".into()],
                json!({}),
            ),
            Action::DeleteChat { chat } => (Method::DELETE, vec!["chat".into(), chat], json!({})),
            Action::DeleteMessage { chat, message } => (
                Method::DELETE,
                vec!["chat".into(), chat, message],
                json!({}),
            ),
            Action::React {
                chat,
                message,
                text,
                reaction,
            } => (
                Method::POST,
                vec!["message".into(), "react".into()],
                json!({"chatGuid":chat,"selectedMessageGuid":message,"selectedMessageText":text,"reaction":reaction,"partIndex":0}),
            ),
            Action::Edit {
                message,
                text,
                original,
            } => (
                Method::POST,
                vec!["message".into(), message, "edit".into()],
                json!({"editedMessage":text,"backwardsCompatibilityMessage":original,"partIndex":0}),
            ),
            Action::Unsend { message } => (
                Method::POST,
                vec!["message".into(), message, "unsend".into()],
                json!({"partIndex":0}),
            ),
            Action::Schedule {
                chat,
                message,
                when,
                private_api,
            } => (
                Method::POST,
                vec!["message".into(), "schedule".into()],
                json!({"type":"send-message","payload":{"chatGuid":chat,"message":message,"method":if private_api {"private-api"} else {"apple-script"}},"scheduledFor":when,"schedule":{}}),
            ),
            Action::DeleteSchedule { id } => (
                Method::DELETE,
                vec!["message".into(), "schedule".into(), id.to_string()],
                json!({}),
            ),
        };
        self.json(
            self.client
                .request(
                    method,
                    self.endpoint(&path.iter().map(String::as_str).collect::<Vec<_>>()),
                )
                .json(&body),
        )
    }

    pub fn search_messages(&self, text: &str, offset: usize) -> ApiResult<Vec<Message>> {
        // Escape LIKE wildcards so searching for '%' or '_' means those characters.
        let query = format!(
            "%{}%",
            text.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        self.json(self.client.post(self.endpoint(&["message","query"])).json(&json!({"with":["chats","attachments","handle"],"where":[{"statement":"message.text LIKE :query ESCAPE '\\'","args":{"query":query}}],"sort":"DESC","offset":offset,"limit":100})))
    }
}

pub fn contact_names(contacts: &[Value]) -> std::collections::HashMap<String, String> {
    let mut names = std::collections::HashMap::new();
    for contact in contacts {
        let name = contact
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{} {}",
                    contact
                        .get("firstName")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    contact
                        .get("lastName")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                )
                .trim()
                .into()
            });
        if name.is_empty() {
            continue;
        }
        for field in ["phoneNumbers", "emails"] {
            if let Some(addresses) = contact.get(field).and_then(Value::as_array) {
                for address in addresses {
                    let address = address.as_str().or_else(|| {
                        address
                            .get("address")
                            .or_else(|| address.get("number"))
                            .and_then(Value::as_str)
                    });
                    if let Some(address) = address {
                        names.insert(normalize_address(address), name.clone());
                    }
                }
            }
        }
    }
    names
}

pub fn normalize_address(address: &str) -> String {
    if address.contains('@') {
        address.trim().to_lowercase()
    } else {
        address.chars().filter(char::is_ascii_digit).collect()
    }
}
