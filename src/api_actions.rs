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
    pub fn server_at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        let mut parts = self
            .server_version
            .trim_start_matches('v')
            .split(['.', '-']);
        let version = (
            parts.next().and_then(|s| s.parse::<u32>().ok()),
            parts.next().and_then(|s| s.parse::<u32>().ok()),
            parts.next().and_then(|s| s.parse::<u32>().ok()),
        );
        matches!(version, (Some(a), Some(b), Some(c)) if (a,b,c) >= (major,minor,patch))
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
    pub fn editable_contact(&self, address: &str) -> ApiResult<Option<Value>> {
        let response = self
            .client
            .get(self.endpoint(&["contact", "capabilities"]))
            .send()
            .map_err(|_| {
                "Could not check contact-editing support. Check your server connection."
            })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("This server needs the contact-editing API patch before contacts can be saved from this app.".into());
        }
        if !response.status().is_success() {
            return Err("Could not check contact-editing support. Check your server connection and password.".into());
        }
        let capabilities: Value = response
            .json()
            .map_err(|_| "Invalid contact-editing capability response.")?;
        if capabilities
            .pointer("/data/localContactNames")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err("This server does not support contact-name editing from clients.".into());
        }
        let key = normalize_address(address);
        let matches = self
            .contacts()?
            .into_iter()
            .filter(|contact| {
                ["phoneNumbers", "emails"].iter().any(|field| {
                    contact
                        .get(field)
                        .and_then(Value::as_array)
                        .is_some_and(|values| {
                            values.iter().any(|value| {
                                value
                                    .as_str()
                                    .or_else(|| {
                                        value
                                            .get("address")
                                            .or_else(|| value.get("number"))
                                            .and_then(Value::as_str)
                                    })
                                    .is_some_and(|value| normalize_address(value) == key)
                            })
                        })
                })
            })
            .collect::<Vec<_>>();
        let mut local = matches
            .iter()
            .filter(|contact| contact.get("sourceType").and_then(Value::as_str) == Some("db"));
        let first = local.next();
        if local.next().is_some() {
            return Err("Multiple server contacts use this address. Resolve the duplicate contacts on the Mac first.".into());
        }
        if let Some(contact) = first {
            return Ok(Some(contact.clone()));
        }
        if !matches.is_empty() {
            return Err("This contact belongs to macOS Contacts. The server cannot edit it remotely; update it in Contacts on your Mac.".into());
        }
        Ok(None)
    }
    pub fn save_contact_name(
        &self,
        contact: Option<&Value>,
        address: &str,
        name: &str,
    ) -> ApiResult<Value> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err("Enter a contact name of 1–200 characters.".into());
        }
        let request = if let Some(contact) = contact {
            if contact.get("sourceType").and_then(Value::as_str) != Some("db") {
                return Err("Only BlueBubbles Server contacts can be edited remotely.".into());
            }
            let id = contact
                .get("id")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or("The server returned an invalid contact ID.")?;
            self.client
                .put(self.endpoint(&["contact", &id.to_string()]))
                .json(&json!({"displayName":name}))
        } else {
            self.client
                .post(self.endpoint(&["contact", "local"]))
                .json(&json!({"displayName":name,"address":address}))
        };
        let value: Value = self.json(request)?;
        let confirmed_id = value.get("id").and_then(Value::as_u64).filter(|id| *id > 0);
        if value.get("sourceType").and_then(Value::as_str) != Some("db")
            || confirmed_id.is_none()
            || contact
                .is_some_and(|original| original.get("id").and_then(Value::as_u64) != confirmed_id)
            || contact_names(std::slice::from_ref(&value))
                .get(&normalize_address(address))
                .map(String::as_str)
                != Some(name)
        {
            return Err("The server did not confirm the contact change. Check its contact list before trying again.".into());
        }
        Ok(value)
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
    for contact in contacts
        .iter()
        .filter(|c| c.get("sourceType").and_then(Value::as_str) != Some("db"))
        .chain(
            contacts
                .iter()
                .filter(|c| c.get("sourceType").and_then(Value::as_str) == Some("db")),
        )
    {
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
