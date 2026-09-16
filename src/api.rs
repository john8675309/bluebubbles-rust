use crate::model::{Chat, Message};
use reqwest::{
    blocking::{multipart, Client, RequestBuilder, Response},
    redirect::Policy,
    Url,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{collections::HashSet, fs::OpenOptions, io::Read, path::Path, time::Duration};

pub type ApiResult<T> = Result<T, String>;

#[derive(Clone)]
pub struct Api {
    pub(crate) client: Client,
    base: Url,
    pub(crate) secret: String,
}

impl Api {
    /// Authenticated Socket.IO endpoint; never log or persist this URL.
    pub fn socket_url(&self) -> String {
        let mut url = self.base.clone();
        let prefix = url.path().trim_end_matches("api/v1/").to_string();
        url.set_path(&format!("{prefix}socket.io/"));
        url.query_pairs_mut().append_pair("guid", &self.secret);
        url.to_string()
    }

    pub fn server_identity(&self) -> String {
        self.base.as_str().trim_end_matches("api/v1/").to_string()
    }
    pub fn new(server: &str, secret: &str) -> ApiResult<Self> {
        crate::initialize_tls();
        let mut base = Url::parse(server.trim())
            .map_err(|_| "Enter a valid http:// or https:// server URL.")?;
        if !matches!(base.scheme(), "https" | "http")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "Use an HTTP(S) server URL without credentials, query parameters, or a fragment."
                    .into(),
            );
        }
        let path = base
            .path()
            .trim_end_matches('/')
            .trim_end_matches("/api/v1")
            .to_owned();
        base.set_path(&format!("{path}/api/v1/"));
        if secret.trim().is_empty() {
            return Err("Enter your server password / authentication key.".into());
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(Policy::none())
            .build()
            .map_err(|_| "Could not initialize the HTTP client.")?;
        Ok(Self {
            client,
            base,
            secret: secret.into(),
        })
    }

    pub(crate) fn endpoint(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("HTTP URL")
            .pop_if_empty()
            .extend(segments);
        url.query_pairs_mut().append_pair("guid", &self.secret);
        url
    }

    pub(crate) fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> ApiResult<T> {
        let response = request.send().map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(response_error(response));
        }
        let value: Value = response
            .json()
            .map_err(|_| "Server returned invalid JSON.".to_string())?;
        if value
            .get("status")
            .and_then(Value::as_i64)
            .is_some_and(|s| s >= 400)
        {
            return Err("The server rejected the operation. Check its logs for details.".into());
        }
        serde_json::from_value(value.get("data").cloned().unwrap_or(Value::Null))
            .map_err(|_| "Unexpected server response format. Check the server version.".into())
    }

    pub fn connect(&self) -> ApiResult<()> {
        self.json::<Value>(self.client.get(self.endpoint(&["server", "info"])))?;
        Ok(())
    }

    pub fn chats(&self, offset: usize) -> ApiResult<Vec<Chat>> {
        self.chat_page(offset, 100)
    }

    fn chat_page(&self, offset: usize, limit: usize) -> ApiResult<Vec<Chat>> {
        self.json(self.client.post(self.endpoint(&["chat", "query"]))
            .json(&json!({"with": ["participants", "lastmessage"], "offset": offset, "limit": limit, "sort": "lastmessage"})))
    }

    /// The server paginates by chat ROWID before sorting each page by activity.
    /// All pages are needed to find recent messages in long-established chats.
    pub fn all_chats(&self) -> ApiResult<Vec<Chat>> {
        let mut chats = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        loop {
            let page = self.chat_page(offset, 1000)?;
            if page.is_empty() {
                break;
            }
            offset += page.len();
            let before = chats.len();
            for chat in page {
                if seen.insert(chat.guid.clone()) {
                    chats.push(chat);
                }
            }
            if chats.len() == before {
                return Err("The server did not advance conversation pagination.".into());
            }
            // Query until empty: some server versions cap page sizes themselves.
        }
        chats.sort_by(|a, b| {
            b.last_activity()
                .cmp(&a.last_activity())
                .then(a.guid.cmp(&b.guid))
        });
        Ok(chats)
    }

    pub fn messages(&self, guid: &str, before: Option<i64>) -> ApiResult<Vec<Message>> {
        self.messages_page(guid, before, 0)
    }

    pub fn messages_page(
        &self,
        guid: &str,
        before: Option<i64>,
        offset: usize,
    ) -> ApiResult<Vec<Message>> {
        let mut request = self.client.post(self.endpoint(&["message", "query"]));
        request = request.json(&json!({"chatGuid": guid, "with": ["handle", "attachments"],
            "sort": "DESC", "limit": 100, "offset": offset, "before": before}));
        self.json(request)
    }

    /// Catch up through every page after an outage, stopping once history overlaps.
    pub fn refresh_messages(&self, guid: &str, known: &HashSet<String>) -> ApiResult<Vec<Message>> {
        let mut page = self.messages(guid, None)?;
        let before = page
            .iter()
            .filter_map(|m| m.date_created)
            .max()
            .map(|t| t.saturating_add(1));
        let mut messages = Vec::new();
        let mut offset = 0;
        loop {
            let done = known.is_empty()
                || page.len() < 100
                || page.iter().any(|m| known.contains(&m.guid));
            offset += page.len();
            let previous_len = messages.len();
            crate::model::merge_messages(&mut messages, page);
            if done {
                return Ok(messages);
            }
            if messages.len() == previous_len {
                return Err("The server did not advance message pagination.".into());
            }
            page = self.messages_page(guid, before, offset)?;
        }
    }

    pub fn send_text(&self, chat: &str, text: &str, temp_guid: &str) -> ApiResult<()> {
        self.json::<Value>(self.client.post(self.endpoint(&["message", "text"]))
            .json(&json!({"chatGuid": chat, "message": text, "tempGuid": temp_guid, "method": "apple-script"})))?;
        Ok(())
    }

    pub fn create_chat(&self, addresses: Vec<String>, text: &str) -> ApiResult<Chat> {
        self.json(self.client.post(self.endpoint(&["chat", "new"]))
            .json(&json!({"addresses": addresses, "message": text, "service": "iMessage", "method": "apple-script"})))
    }

    pub fn send_attachment(&self, chat: &str, path: &Path, temp_guid: &str) -> ApiResult<()> {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid filename.")?;
        if !path.is_file() {
            return Err("Choose an existing regular file.".into());
        }
        let form = multipart::Form::new()
            .text("chatGuid", chat.to_string())
            .text("tempGuid", temp_guid.to_string())
            .text("method", "apple-script")
            .text("name", name.to_string())
            .file("attachment", path)
            .map_err(|_| "Could not read the attachment file.")?;
        self.json::<Value>(
            self.client
                .post(self.endpoint(&["message", "attachment"]))
                .timeout(Duration::from_secs(300))
                .multipart(form),
        )?;
        Ok(())
    }

    pub fn download(&self, guid: &str, path: &Path) -> ApiResult<()> {
        let mut response = self
            .client
            .get(self.endpoint(&["attachment", guid, "download"]))
            .timeout(Duration::from_secs(300))
            .send()
            .map_err(network_error)?;
        if !response.status().is_success() {
            return Err(response_error(response));
        }
        // create_new prevents overwriting existing files (including symlinks).
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|_| {
            "Cannot create that file. Choose a writable location and a new filename."
        })?;
        if response.copy_to(&mut file).is_err() {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err("Download interrupted. The incomplete file was removed.".into());
        }
        Ok(())
    }
}

fn network_error(error: reqwest::Error) -> String {
    // reqwest errors include the authenticated URL; never expose them in UI/logs.
    if error.is_timeout() {
        "Request timed out. Check your connection and server."
    } else if error.is_connect() {
        "Cannot reach the server. Check its URL, network, and TLS certificate."
    } else {
        "Network request failed. Check your connection and server."
    }
    .into()
}

fn status_error(code: u16) -> String {
    match code {
        401 | 403 => "Authentication failed. Check your server password / key.".into(),
        301..=399 => "The server redirected the request. Enter its final server URL.".into(),
        _ => format!("Server returned HTTP {code}. Check the server logs."),
    }
}

fn response_error(response: Response) -> String {
    let status = response.status().as_u16();
    let cloudflare = response
        .headers()
        .get("server")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("cloudflare"))
        || response.headers().contains_key("cf-ray");
    if status != 530 || !cloudflare {
        return status_error(status);
    }

    // Inspect a bounded error page only for known codes. Never display its raw
    // HTML, URLs, or diagnostic text: these can contain credentials or IPs.
    let mut bytes = Vec::new();
    let _ = response.take(64 * 1024).read_to_end(&mut bytes);
    let body: String = String::from_utf8_lossy(&bytes)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let is_tunnel_error = ["errorCode:1033", "\"errorCode\":1033", ">1033</span>"]
        .iter()
        .any(|marker| body.contains(marker));
    if is_tunnel_error {
        "Cloudflare tunnel is offline (HTTP 530 / error 1033). On your Mac, check BlueBubbles Server and its tunnel, then enter its current server URL. On the same network, you can use the server's local address instead.".into()
    } else {
        "Cloudflare cannot resolve or reach the server (HTTP 530). Check the tunnel and current server URL in BlueBubbles Server on your Mac.".into()
    }
}
