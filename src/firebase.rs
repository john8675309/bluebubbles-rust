//! Desktop Firebase URL discovery. Only public client configuration is retained.
use crate::api::{Api, ApiResult};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Read, path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FirebaseConfig {
    pub project_id: String,
    pub database_url: Option<String>,
}

impl FirebaseConfig {
    pub fn from_google_services(value: &Value) -> ApiResult<Self> {
        if value.get("private_key").is_some()
            || value.get("type").and_then(Value::as_str) == Some("service_account")
        {
            return Err(
                "Choose google-services.json (client configuration), not a service-account key."
                    .into(),
            );
        }
        let info = value
            .get("project_info")
            .ok_or("Missing project_info in google-services.json.")?;
        let config = Self {
            project_id: info
                .get("project_id")
                .and_then(Value::as_str)
                .ok_or("Missing Firebase project ID.")?
                .into(),
            database_url: info
                .get("firebase_url")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        };
        config.endpoint()?;
        Ok(config)
    }

    pub fn import(path: &Path) -> ApiResult<Self> {
        let file =
            std::fs::File::open(path).map_err(|_| "Cannot read Firebase client configuration.")?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read Firebase client configuration.")?;
        if bytes.len() > 1024 * 1024 {
            return Err("Firebase client configuration is too large.".into());
        }
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| "Firebase client configuration is not valid JSON.")?;
        Self::from_google_services(&value)
    }

    pub fn endpoint(&self) -> ApiResult<Url> {
        if self.project_id.is_empty()
            || self.project_id.len() > 100
            || !self
                .project_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("Invalid Firebase project ID.".into());
        }
        if let Some(database) = &self.database_url {
            let mut url = Url::parse(database).map_err(|_| "Invalid Firebase database URL.")?;
            let host = url.host_str().unwrap_or("");
            if url.scheme() != "https"
                || !(host.ends_with(".firebaseio.com") || host.ends_with(".firebasedatabase.app"))
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.port().is_some()
                || !matches!(url.path(), "" | "/")
            {
                return Err("Use the HTTPS root URL of a Firebase Realtime Database.".into());
            }
            url.set_path("/config/serverUrl.json");
            Ok(url)
        } else {
            Url::parse(&format!("https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/server/config", self.project_id)).map_err(|_| "Invalid Firebase project ID.".into())
        }
    }

    pub fn discover(&self) -> ApiResult<String> {
        crate::initialize_tls();
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(8))
            .redirect(Policy::none())
            .build()
            .map_err(|_| "Cannot initialize Firebase connection.")?;
        self.discover_at(&client, self.endpoint()?)
    }

    fn discover_at(&self, client: &Client, endpoint: Url) -> ApiResult<String> {
        // Never attach the Mac password, service-account keys, or user credentials.
        let response = client
            .get(endpoint)
            .send()
            .map_err(|_| "Cannot reach Firebase. Check your network connection.")?;
        match response.status().as_u16() {
            200 => {},
            401 | 403 => return Err("Firebase denied access to the server URL. Check this project's BlueBubbles database configuration.".into()),
            404 => return Err("Firebase has no server configuration. Check Firebase setup on your Mac.".into()),
            status => return Err(format!("Firebase lookup returned HTTP {status}.")),
        }
        let mut bytes = Vec::new();
        response
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Firebase response was interrupted.")?;
        if bytes.len() > 64 * 1024 {
            return Err("Firebase response is too large.".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Firebase returned invalid JSON.")?;
        let address = if self.database_url.is_some() {
            value.as_str()
        } else {
            value
                .pointer("/fields/serverUrl/stringValue")
                .and_then(Value::as_str)
        }
        .ok_or("Firebase does not contain a valid server URL.")?;
        validate_server_url(address)
    }
}

fn validate_server_url(address: &str) -> ApiResult<String> {
    let parsed =
        Url::parse(address.trim()).map_err(|_| "Firebase contains an invalid server URL.")?;
    // Automatic rediscovery must not downgrade a tunnel's TLS connection.
    if parsed.scheme() != "https" {
        return Err(
            "Firebase returned a non-HTTPS URL. Enter a local HTTP address manually if needed."
                .into(),
        );
    }
    Api::new(address, "validation-only").map_err(|_| "Firebase contains an invalid server URL.")?;
    Ok(address.trim().trim_end_matches('/').to_string())
}

impl Api {
    pub fn firebase_config(&self) -> ApiResult<FirebaseConfig> {
        let value = self.json(self.client.get(self.endpoint(&["fcm", "client"])))?;
        FirebaseConfig::from_google_services(&value)
    }

    pub fn at_server(&self, server: &str) -> ApiResult<Self> {
        Self::new(server, &self.secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn firestore_and_realtime_lookup_decode_wire_responses_without_credentials() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        crate::initialize_tls();
        for (database_url, body) in [
            (
                None,
                json!({"fields":{"serverUrl":{"stringValue":"https://new-tunnel.example"}}}),
            ),
            (
                Some("https://bb-default-rtdb.firebaseio.com".to_string()),
                json!("https://new-tunnel.example"),
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint =
                Url::parse(&format!("http://{}/lookup", listener.local_addr().unwrap())).unwrap();
            let worker = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.starts_with("get /lookup http/1.1"));
                assert!(!request.contains("authorization:"));
                assert!(!request.contains("guid="));
                let body = body.to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            });
            let config = FirebaseConfig {
                project_id: "bb-example".into(),
                database_url,
            };
            let client = Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap();
            assert_eq!(
                config.discover_at(&client, endpoint).unwrap(),
                "https://new-tunnel.example"
            );
            worker.join().unwrap();
        }
    }
    #[test]
    fn reads_public_client_config_and_both_database_endpoints() {
        let mut source = json!({"project_info":{"project_id":"bb-example"}, "client":[{"api_key":[{"current_key":"not-persisted"}]}]});
        let firestore = FirebaseConfig::from_google_services(&source).unwrap();
        assert!(firestore
            .endpoint()
            .unwrap()
            .as_str()
            .ends_with("/projects/bb-example/databases/(default)/documents/server/config"));
        assert!(!serde_json::to_string(&firestore)
            .unwrap()
            .contains("not-persisted"));
        for host in [
            "bb-example-default-rtdb.firebaseio.com",
            "bb-example-default-rtdb.europe-west1.firebasedatabase.app",
        ] {
            source["project_info"]["firebase_url"] = json!(format!("https://{host}"));
            assert_eq!(
                FirebaseConfig::from_google_services(&source)
                    .unwrap()
                    .endpoint()
                    .unwrap()
                    .as_str(),
                format!("https://{host}/config/serverUrl.json")
            );
        }
    }

    #[test]
    fn rejects_keys_untrusted_database_hosts_and_unsafe_discovered_urls() {
        assert!(FirebaseConfig::from_google_services(
            &json!({"type":"service_account","private_key":"secret"})
        )
        .is_err());
        for url in [
            "http://bb.firebaseio.com",
            "https://firebaseio.com.evil.test",
            "https://bb.firebaseio.com/?auth=secret",
            "https://user:secret@bb.firebaseio.com",
            "https://bb.firebaseio.com/path",
        ] {
            assert!(FirebaseConfig {
                project_id: "bb-example".into(),
                database_url: Some(url.into())
            }
            .endpoint()
            .is_err());
        }
        for url in [
            "http://example.com",
            "file:///tmp/config",
            "https://user:secret@example.com",
            "https://example.com/?guid=secret",
        ] {
            assert!(validate_server_url(url).is_err());
        }
        assert_eq!(
            validate_server_url(" https://new-tunnel.example/ ").unwrap(),
            "https://new-tunnel.example"
        );
    }
}
