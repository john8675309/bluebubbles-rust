use bluebubbles_linux::{
    api::Api,
    model::{merge_messages, Chat, Message},
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

#[test]
fn conversations_are_globally_sorted_across_server_pages() {
    // Simulate a capped server page: an old-created chat on page two has
    // the newest message. Last-message sorting only happens within each page.
    let responses = [
        json!({"data":[{"guid":"recent-chat","lastMessage":{"guid":"m1","dateCreated":20}}]}),
        json!({"data":[{"guid":"old-active-chat","lastMessage":{"guid":"m2","dateCreated":30}},
            {"guid":"delivered-only","lastMessage":{"guid":"m3","dateDelivered":25}},
            {"guid":"empty-chat"}]}),
        json!({"data":[]}),
    ];
    let (url, rx, worker) = server_many(
        responses
            .into_iter()
            .map(|body| (200, serde_json::to_vec(&body).unwrap()))
            .collect(),
    );
    let chats = Api::new(&url, "secret").unwrap().all_chats().unwrap();
    assert_eq!(
        chats
            .iter()
            .map(|chat| chat.guid.as_str())
            .collect::<Vec<_>>(),
        vec![
            "old-active-chat",
            "delivered-only",
            "recent-chat",
            "empty-chat"
        ]
    );
    for offset in [0, 1, 4] {
        let request = rx.recv().unwrap();
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["offset"], offset);
        assert_eq!(body["limit"], 1000);
    }
    worker.join().unwrap();
}

#[test]
fn repeated_conversation_pages_fail_instead_of_looping_forever() {
    let body = serde_json::to_vec(&json!({"data":[{"guid":"chat"}]})).unwrap();
    let (url, rx, worker) = server_many(vec![(200, body.clone()), (200, body)]);
    assert!(Api::new(&url, "secret")
        .unwrap()
        .all_chats()
        .unwrap_err()
        .contains("did not advance"));
    rx.recv().unwrap();
    rx.recv().unwrap();
    worker.join().unwrap();
}

fn server(status: u16, body: Vec<u8>) -> (String, mpsc::Receiver<Request>, thread::JoinHandle<()>) {
    server_many(vec![(status, body)])
}

fn server_many(
    responses: Vec<(u16, Vec<u8>)>,
) -> (String, mpsc::Receiver<Request>, thread::JoinHandle<()>) {
    server_headers(responses, "")
}

fn server_headers(
    responses: Vec<(u16, Vec<u8>)>,
    headers: &'static str,
) -> (String, mpsc::Receiver<Request>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let (end, length) = loop {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            let target = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            tx.send(Request {
                method: String::from_utf8_lossy(&bytes[..end])
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_string(),
                target,
                body: bytes[end..end + length].to_vec(),
            })
            .unwrap();
            write!(socket, "HTTP/1.1 {status} Test\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            socket.write_all(&body).unwrap();
        }
    });
    (url, rx, handle)
}

#[test]
fn private_message_options_and_actions_match_server_contract() {
    use bluebubbles_linux::api_actions::{Action, SendOptions};
    let response = serde_json::to_vec(&json!({"data":{}})).unwrap();
    let (url, rx, worker) = server_many(vec![(200, response); 4]);
    let api = Api::new(&url, "secret").unwrap();
    api.send_with_options(
        "chat",
        "reply",
        "temp",
        SendOptions {
            private_api: true,
            reply: Some("original".into()),
            subject: Some("Subject".into()),
            effect: None,
        },
    )
    .unwrap();
    api.perform(Action::React {
        chat: "chat".into(),
        message: "original".into(),
        text: "hello".into(),
        reaction: "love".into(),
    })
    .unwrap();
    api.perform(Action::Edit {
        message: "original".into(),
        text: "changed".into(),
        original: "hello".into(),
    })
    .unwrap();
    api.perform(Action::Typing {
        chat: "chat".into(),
        active: false,
    })
    .unwrap();
    let sent = rx.recv().unwrap();
    let body: Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(sent.method, "POST");
    assert_eq!(body["method"], "private-api");
    assert_eq!(body["selectedMessageGuid"], "original");
    assert_eq!(body["subject"], "Subject");
    let reaction = rx.recv().unwrap();
    assert!(reaction.target.starts_with("/api/v1/message/react?"));
    let body: Value = serde_json::from_slice(&reaction.body).unwrap();
    assert_eq!(body["reaction"], "love");
    assert_eq!(body["partIndex"], 0);
    let edited = rx.recv().unwrap();
    assert!(edited.target.starts_with("/api/v1/message/original/edit?"));
    let body: Value = serde_json::from_slice(&edited.body).unwrap();
    assert_eq!(body["editedMessage"], "changed");
    let typing = rx.recv().unwrap();
    assert_eq!(typing.method, "DELETE");
    assert!(typing.target.starts_with("/api/v1/chat/chat/typing?"));
    worker.join().unwrap();
}

#[test]
fn contacts_and_server_capabilities_decode_actual_server_shapes() {
    use bluebubbles_linux::api_actions::{contact_names, ServerInfo};
    let names = contact_names(&[
        json!({"displayName":"Alex", "phoneNumbers":[{"address":"+1 (555) 123-4567"}],"emails":[{"address":"Alex@Example.com"}]}),
    ]);
    assert_eq!(names["15551234567"], "Alex");
    assert_eq!(names["alex@example.com"], "Alex");
    let mut info: ServerInfo = serde_json::from_value(json!({"server_version":"1.9.9","os_version":"14.0","private_api":true,"helper_connected":false})).unwrap();
    assert!(!info.private_available());
    info.helper_connected = true;
    assert!(info.private_available());
    assert!(info.macos_at_least(13));
    assert!(!info.macos_at_least(15));
}

#[test]
fn reaction_names_load_in_both_chat_previews_and_message_history() {
    use bluebubbles_linux::model::AssociatedMessageType;
    for kind in [
        json!("love"),
        json!("-like"),
        json!("sticker"),
        json!("future-reaction"),
        json!(2000),
        Value::Null,
    ] {
        let message = json!({"guid":"reaction", "associatedMessageGuid":"p:0/original",
            "associatedMessageType":kind, "text":"Reacted to a message", "dateCreated":100});
        let chats = json!({"data":[{"guid":"chat", "lastMessage":message}]});
        let history = json!({"data":[message]});
        let (url, rx, worker) = server_many(vec![
            (200, serde_json::to_vec(&chats).unwrap()),
            (200, serde_json::to_vec(&history).unwrap()),
        ]);
        let api = Api::new(&url, "secret").unwrap();
        let chats = api.chats(0).unwrap();
        let history = api.messages("chat", None).unwrap();
        let expected = match &kind {
            Value::String(name) => Some(AssociatedMessageType::Name(name.clone())),
            Value::Number(number) => Some(AssociatedMessageType::Code(number.as_i64().unwrap())),
            _ => None,
        };
        assert_eq!(
            chats[0]
                .last_message
                .as_ref()
                .unwrap()
                .associated_message_type,
            expected
        );
        assert_eq!(history[0].associated_message_type, expected);
        assert_eq!(
            serde_json::to_value(&history[0]).unwrap()["associatedMessageType"],
            kind
        );
        rx.recv().unwrap();
        rx.recv().unwrap();
        worker.join().unwrap();
    }
}

#[test]
fn cloudflare_offline_tunnel_has_actionable_error_without_leaking_body() {
    for body in ["<script>errorCode: 1033</script>", "<span>1033</span>"] {
        let body = format!("{body} secret-password https://private.example/?guid=secret-password")
            .into_bytes();
        let (url, rx, worker) = server_headers(
            vec![(530, body.clone()), (530, body)],
            "Server: cloudflare\r\n",
        );
        let api = Api::new(&url, "secret-password").unwrap();
        let connect_error = api.connect().unwrap_err();
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let download_error = api.download("attachment", &path).unwrap_err();
        assert!(!path.exists());
        for error in [connect_error, download_error] {
            assert!(error.contains("tunnel is offline"));
            assert!(error.contains("1033"));
            assert!(error.contains("current server URL"));
            assert!(!error.contains("secret-password"));
            assert!(!error.contains("private.example"));
        }
        rx.recv().unwrap();
        rx.recv().unwrap();
        worker.join().unwrap();
    }
}

#[test]
fn other_530_responses_are_not_misidentified_as_offline_tunnels() {
    for (headers, body, expected) in [
        (
            "Server: cloudflare\r\n",
            "errorCode: 1016",
            "cannot resolve or reach",
        ),
        ("", "errorCode: 1033", "Server returned HTTP 530"),
    ] {
        let (url, rx, worker) = server_headers(vec![(530, body.as_bytes().to_vec())], headers);
        let error = Api::new(&url, "password").unwrap().connect().unwrap_err();
        assert!(error.contains(expected));
        assert!(!error.contains("tunnel is offline"));
        rx.recv().unwrap();
        worker.join().unwrap();
    }
}

#[test]
fn refresh_catches_up_more_than_one_page_after_disconnect() {
    let page = |range: std::ops::Range<usize>| {
        let messages: Vec<_> = range
            .rev()
            .map(|i| json!({"guid": format!("m{i}"), "dateCreated": i as i64}))
            .collect();
        (200, json!({"data": messages}).to_string().into_bytes())
    };
    let (url, rx, worker) = server_many(vec![page(200..300), page(100..200), page(0..100)]);
    let known = ["m50".into()].into_iter().collect();
    let messages = Api::new(&url, "password")
        .unwrap()
        .refresh_messages("chat", &known)
        .unwrap();
    assert_eq!(messages.len(), 300);
    assert_eq!(messages[0].guid, "m0");
    assert_eq!(messages[299].guid, "m299");
    for offset in [0, 100, 200] {
        let request: Value = serde_json::from_slice(&rx.recv().unwrap().body).unwrap();
        assert_eq!(request["offset"], offset);
        if offset > 0 {
            assert_eq!(request["before"], 300);
        }
    }
    worker.join().unwrap();
}

#[test]
fn pagination_detects_a_server_that_repeats_the_same_page() {
    let messages: Vec<_> = (0..100)
        .map(|i| json!({"guid": format!("m{i}"), "dateCreated": 100}))
        .collect();
    let body = json!({"data": messages}).to_string().into_bytes();
    let (url, rx, worker) = server_many(vec![(200, body.clone()), (200, body)]);
    let known = ["older".into()].into_iter().collect();
    let error = Api::new(&url, "password")
        .unwrap()
        .refresh_messages("chat", &known)
        .unwrap_err();
    assert!(error.contains("pagination"));
    rx.recv().unwrap();
    rx.recv().unwrap();
    worker.join().unwrap();
}

#[test]
fn chat_query_matches_upstream_and_encodes_secret() {
    let response =
        json!({"status": 200, "data": [{"guid": "iMessage;-;friend", "displayName": null,
        "participants": [{"address": "friend@example.com"}], "lastMessage": null}]})
        .to_string()
        .into_bytes();
    let (url, rx, worker) = server(200, response);
    let api = Api::new(&format!("{url}/proxy/api/v1/"), "secret &+=").unwrap();
    let chats = api.chats(100).unwrap();
    assert_eq!(chats[0].title(), "friend@example.com");
    let request = rx.recv().unwrap();
    let url = reqwest::Url::parse(&format!("http://localhost{}", request.target)).unwrap();
    assert_eq!(url.path(), "/proxy/api/v1/chat/query");
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        vec![("guid".into(), "secret &+=".into())]
    );
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["offset"], 100);
    assert_eq!(body["sort"], "lastmessage");
    assert_eq!(body["with"], json!(["participants", "lastmessage"]));
    worker.join().unwrap();
}

#[test]
fn text_send_preserves_unicode_and_idempotency_key() {
    let (url, rx, worker) = server(200, br#"{"status":200,"data":{"guid":"sent"}}"#.to_vec());
    Api::new(&url, "password")
        .unwrap()
        .send_text("iMessage;+;group", "Hello 👋\n第二行", "unique-temp-id")
        .unwrap();
    let request = rx.recv().unwrap();
    assert!(request.target.starts_with("/api/v1/message/text?"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({
            "chatGuid": "iMessage;+;group", "message": "Hello 👋\n第二行", "tempGuid": "unique-temp-id", "method": "apple-script"
        })
    );
    worker.join().unwrap();
}

#[test]
fn messages_include_attachments_and_history_cursor() {
    let (url, rx, worker) = server(
        200,
        br#"{"data":[{"guid":"m1","isFromMe":false,"attachments":null}]}"#.to_vec(),
    );
    let messages = Api::new(&url, "password")
        .unwrap()
        .messages("chat", Some(1750000000123))
        .unwrap();
    assert!(messages[0].attachments.is_empty());
    let body: Value = serde_json::from_slice(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["chatGuid"], "chat");
    assert_eq!(body["before"], 1750000000123i64);
    assert_eq!(body["with"], json!(["handle", "attachments"]));
    worker.join().unwrap();
}

#[test]
fn authentication_and_server_errors_do_not_expose_secrets() {
    for status in [401, 403, 302, 500] {
        let (url, rx, worker) = server(status, b"super-secret error content".to_vec());
        let error = Api::new(&url, "super-secret")
            .unwrap()
            .connect()
            .unwrap_err();
        assert!(!error.contains("super-secret"));
        assert!(!error.contains(&url));
        if status == 401 || status == 403 {
            assert!(error.contains("Authentication failed"));
        }
        rx.recv().unwrap();
        worker.join().unwrap();
    }
}

#[test]
fn malformed_or_rejected_success_responses_are_errors() {
    for body in [
        b"not JSON".to_vec(),
        br#"{"status":500,"data":[]}"#.to_vec(),
        br#"{"data":{}}"#.to_vec(),
    ] {
        let (url, rx, worker) = server(200, body);
        assert!(Api::new(&url, "password").unwrap().chats(0).is_err());
        rx.recv().unwrap();
        worker.join().unwrap();
    }
}

#[test]
fn invalid_server_urls_are_rejected() {
    for url in [
        "file:///tmp/secret",
        "ftp://example.com",
        "https://user:pass@example.com",
        "https://example.com?guid=secret",
        "https://example.com/#fragment",
        "not a url",
    ] {
        assert!(Api::new(url, "password").is_err(), "{url}");
    }
    assert!(Api::new("https://example.com", " ").is_err());
}

#[test]
fn create_chat_uses_the_upstream_response_shape() {
    let (url, rx, worker) = server(
        200,
        br#"{"data":{"guid":"new-chat","participants":[],"messages":[{"guid":"initial"}]}}"#
            .to_vec(),
    );
    let chat = Api::new(&url, "password")
        .unwrap()
        .create_chat(vec!["friend@example.com".into()], "Hello")
        .unwrap();
    assert_eq!(chat.guid, "new-chat");
    let body: Value = serde_json::from_slice(&rx.recv().unwrap().body).unwrap();
    assert_eq!(body["addresses"], json!(["friend@example.com"]));
    assert_eq!(body["service"], "iMessage");
    worker.join().unwrap();
}

#[test]
fn attachment_upload_is_multipart_and_download_never_overwrites() {
    let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&folder).unwrap();
    let source = folder.join("example.txt");
    std::fs::write(&source, "file contents").unwrap();
    let (url, rx, worker) = server(200, br#"{"data":null}"#.to_vec());
    Api::new(&url, "password")
        .unwrap()
        .send_attachment("chat-guid", &source, "temp-guid")
        .unwrap();
    let body = String::from_utf8(rx.recv().unwrap().body).unwrap();
    for expected in [
        "name=\"attachment\"",
        "filename=\"example.txt\"",
        "file contents",
        "chat-guid",
        "temp-guid",
    ] {
        assert!(body.contains(expected));
    }
    worker.join().unwrap();
    let destination = folder.join("download.txt");
    for attempt in 0..2 {
        let (url, rx, worker) = server(200, b"downloaded".to_vec());
        let result = Api::new(&url, "password")
            .unwrap()
            .download("a/b?c", &destination);
        assert_eq!(result.is_ok(), attempt == 0);
        assert_eq!(std::fs::read(&destination).unwrap(), b"downloaded");
        assert!(rx
            .recv()
            .unwrap()
            .target
            .starts_with("/api/v1/attachment/a%2Fb%3Fc/download?"));
        worker.join().unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn overlapping_history_updates_receipts_without_duplicates() {
    let mut messages = vec![Message {
        guid: "new".into(),
        date_created: Some(20),
        ..Default::default()
    }];
    merge_messages(
        &mut messages,
        vec![
            Message {
                guid: "old".into(),
                date_created: Some(10),
                ..Default::default()
            },
            Message {
                guid: "new".into(),
                date_created: Some(20),
                date_read: Some(30),
                ..Default::default()
            },
        ],
    );
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].guid, "old");
    assert_eq!(messages[1].delivery(), "Read");
    let chat: Chat =
        serde_json::from_value(json!({"guid":"fallback", "displayName":" ", "participants":null}))
            .unwrap();
    assert_eq!(chat.title(), "fallback");
}

#[test]
fn private_chat_read_group_and_unsend_requests_match_server() {
    use bluebubbles_linux::api_actions::Action;
    let response = serde_json::to_vec(&json!({"data":{}})).unwrap();
    let (url, rx, worker) = server_many(vec![(200, response); 7]);
    let api = Api::new(&url, "secret").unwrap();
    let actions = [
        (
            Action::Read {
                chat: "iMessage;+;group".into(),
                read: true,
            },
            "POST",
            "/read",
            json!({}),
        ),
        (
            Action::Read {
                chat: "iMessage;+;group".into(),
                read: false,
            },
            "POST",
            "/unread",
            json!({}),
        ),
        (
            Action::Rename {
                chat: "iMessage;+;group".into(),
                name: "Friends".into(),
            },
            "PUT",
            "group",
            json!({"displayName":"Friends"}),
        ),
        (
            Action::Participant {
                chat: "iMessage;+;group".into(),
                address: "alex@example.com".into(),
                add: true,
            },
            "POST",
            "/participant/add",
            json!({"address":"alex@example.com"}),
        ),
        (
            Action::Participant {
                chat: "iMessage;+;group".into(),
                address: "alex@example.com".into(),
                add: false,
            },
            "POST",
            "/participant/remove",
            json!({"address":"alex@example.com"}),
        ),
        (
            Action::Leave {
                chat: "iMessage;+;group".into(),
            },
            "POST",
            "/leave",
            json!({}),
        ),
        (
            Action::Unsend {
                message: "message".into(),
            },
            "POST",
            "/unsend",
            json!({"partIndex":0}),
        ),
    ];
    for (action, method, suffix, expected) in actions {
        api.perform(action).unwrap();
        let request = rx.recv().unwrap();
        assert_eq!(request.method, method);
        assert!(request.target.split('?').next().unwrap().ends_with(suffix));
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap(),
            expected
        );
    }
    worker.join().unwrap();
}

#[test]
fn private_creation_and_attachment_choose_private_api_explicitly() {
    let (url, rx, worker) = server_many(vec![
        (
            200,
            serde_json::to_vec(&json!({"data":{"guid":"iMessage;+;new"}})).unwrap(),
        ),
        (200, serde_json::to_vec(&json!({"data":{}})).unwrap()),
    ]);
    let api = Api::new(&url, "secret").unwrap();
    api.create_chat_with_method(vec!["alex@example.com".into()], "hello", true)
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&rx.recv().unwrap().body).unwrap()["method"],
        "private-api"
    );
    let path = std::env::temp_dir().join(format!("bb-private-upload-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, "private attachment").unwrap();
    api.send_attachment_with_method("iMessage;+;new", &path, "temp", true)
        .unwrap();
    let request = rx.recv().unwrap();
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains("name=\"method\"\r\n\r\nprivate-api"));
    assert!(body.contains("private attachment"));
    std::fs::remove_file(path).unwrap();
    worker.join().unwrap();
}

#[test]
fn stale_history_cannot_restore_an_unsent_message() {
    let old: Message =
        serde_json::from_value(json!({"guid":"m","text":null,"dateEdited":30,"dateRetracted":30}))
            .unwrap();
    let stale: Message =
        serde_json::from_value(json!({"guid":"m","text":"secret","dateCreated":10})).unwrap();
    let mut history = vec![old];
    merge_messages(&mut history, vec![stale]);
    assert!(history[0].is_unsent());
    assert_eq!(history[0].preview(), "Message unsent");
}

#[test]
fn inline_media_downloads_are_authenticated_and_video_is_rewound() {
    let (url, rx, worker) = server_many(vec![
        (200, b"image fixture".to_vec()),
        (200, b"video fixture".to_vec()),
    ]);
    let api = Api::new(&url, "secret").unwrap();
    assert_eq!(api.image_preview("image-id").unwrap(), b"image fixture");
    let request = rx.recv().unwrap();
    assert!(request.target.contains("/attachment/image-id/download?"));
    assert!(request.target.contains("guid=secret"));
    assert!(request.target.contains("width=1200"));
    let mut file = tempfile::tempfile().unwrap();
    api.video_file(
        "video-id",
        &mut file,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"video fixture");
    assert!(rx.recv().unwrap().target.contains("guid=secret"));
    worker.join().unwrap();
}

#[test]
fn animated_image_requests_original_bytes_to_preserve_frames() {
    let (url, rx, worker) = server_many(vec![(200, b"GIF89a fixture".to_vec())]);
    assert_eq!(
        Api::new(&url, "secret")
            .unwrap()
            .animated_image("gif-id")
            .unwrap(),
        b"GIF89a fixture"
    );
    let request = rx.recv().unwrap();
    assert!(request.target.contains("original=true"));
    assert!(request.target.contains("guid=secret"));
    assert!(!request.target.contains("width="));
    worker.join().unwrap();
}

#[test]
fn server_contact_edit_uses_exact_id_and_preserves_addresses() {
    let old = json!({"id":7,"sourceType":"db","displayName":"Old name","firstName":"Old","phoneNumbers":[{"address":"+15551234567"}],"emails":[{"address":"other@example.com"}]});
    let mut saved = old.clone();
    saved["displayName"] = json!("New name");
    let (url, rx, worker) = server_many(vec![
        (
            200,
            serde_json::to_vec(&json!({"data":{"localContactNames":true}})).unwrap(),
        ),
        (200, serde_json::to_vec(&json!({"data":[old]})).unwrap()),
        (200, serde_json::to_vec(&json!({"data":saved})).unwrap()),
    ]);
    let api = Api::new(&url, "secret").unwrap();
    let contact = api.editable_contact("+1 (555) 123-4567").unwrap().unwrap();
    assert_eq!(
        api.save_contact_name(Some(&contact), "+15551234567", "New name")
            .unwrap()["displayName"],
        "New name"
    );
    assert!(rx.recv().unwrap().target.contains("contact/capabilities"));
    rx.recv().unwrap();
    let request = rx.recv().unwrap();
    assert_eq!(request.method, "PUT");
    assert!(request.target.contains("/contact/7?"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"displayName":"New name"})
    );
    worker.join().unwrap();
}
#[test]
fn stock_server_and_native_mac_contacts_cannot_be_renamed() {
    let (url, rx, worker) = server_many(vec![(404, b"{}".to_vec())]);
    assert!(Api::new(&url, "secret")
        .unwrap()
        .editable_contact("a@example.com")
        .unwrap_err()
        .contains("patch"));
    rx.recv().unwrap();
    worker.join().unwrap();
    let (url,rx,worker)=server_many(vec![(200,serde_json::to_vec(&json!({"data":{"localContactNames":true}})).unwrap()),(200,serde_json::to_vec(&json!({"data":[{"id":"mac-id","sourceType":"api","displayName":"Mac name","emails":[{"address":"a@example.com"}]}]})).unwrap())]);
    assert!(Api::new(&url, "secret")
        .unwrap()
        .editable_contact("a@example.com")
        .unwrap_err()
        .contains("macOS Contacts"));
    rx.recv().unwrap();
    rx.recv().unwrap();
    worker.join().unwrap();
}

#[test]
fn new_server_contact_uses_dedicated_create_and_confirmation() {
    let response = json!({"data":{"id":8,"sourceType":"db","displayName":"New contact","emails":[{"address":"new@example.com"}]}});
    let (url, rx, worker) = server_many(vec![(200, serde_json::to_vec(&response).unwrap())]);
    assert_eq!(
        Api::new(&url, "secret")
            .unwrap()
            .save_contact_name(None, "new@example.com", "New contact")
            .unwrap()["id"],
        8
    );
    let request = rx.recv().unwrap();
    assert_eq!(request.method, "POST");
    assert!(request.target.contains("/contact/local?"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"displayName":"New contact","address":"new@example.com"})
    );
    worker.join().unwrap();
    let (url, rx, worker) = server_many(vec![(200, serde_json::to_vec(&response).unwrap())]);
    let wrong = json!({"id":7,"sourceType":"db"});
    assert!(Api::new(&url, "secret")
        .unwrap()
        .save_contact_name(Some(&wrong), "new@example.com", "New contact")
        .unwrap_err()
        .contains("did not confirm"));
    rx.recv().unwrap();
    worker.join().unwrap();
}
