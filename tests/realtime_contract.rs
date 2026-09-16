use bluebubbles_linux::{
    api::Api,
    realtime::{LiveConnection, LiveEvent},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn authenticates_socket_path_and_receives_live_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/prefix/api/v1", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let packets = Arc::new(Mutex::new(std::collections::VecDeque::<String>::new()));
    let (requests_tx, requests_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        while !stopping.load(Ordering::Relaxed) {
            let Ok((mut socket, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            let packets = packets.clone();
            let requests = requests_tx.clone();
            thread::spawn(move || {
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let (end, length) = loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
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
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let line = headers.lines().next().unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap();
                let target = parts.next().unwrap();
                let parsed = reqwest::Url::parse(&format!("http://localhost{target}")).unwrap();
                let auth = parsed
                    .query_pairs()
                    .any(|(k, v)| k == "guid" && v == "socket password&?");
                requests.send((parsed.path().to_string(), auth)).unwrap();
                let response = if method == "POST" {
                    let body = &bytes[end..end + length];
                    if body == b"40" {
                        let mut queue = packets.lock().unwrap();
                        queue.push_back("40{\"sid\":\"namespace\"}".into());
                        queue.push_back("42[\"new-message\",{\"guid\":\"live-1\",\"text\":\"Arrived live\",\"chats\":[{\"guid\":\"chat\"}]}]".into());
                    }
                    "ok".to_string()
                } else if !parsed.query_pairs().any(|(k, _)| k == "sid") {
                    "0{\"sid\":\"engine\",\"upgrades\":[],\"pingInterval\":25000,\"pingTimeout\":20000,\"maxPayload\":1000000}".into()
                } else {
                    thread::sleep(Duration::from_millis(50));
                    packets
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| "2".into())
                };
                let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response);
            });
        }
    });
    let (tx, rx) = mpsc::channel();
    let live = LiveConnection::start(Api::new(&url, "socket password&?").unwrap(), move |event| {
        let _ = tx.send(event);
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connected = false;
    let mut received = false;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(LiveEvent::Connected) => connected = true,
            Ok(LiveEvent::Data { name, data }) => {
                assert_eq!(name, "new-message");
                assert_eq!(data["guid"], "live-1");
                received = true;
                break;
            }
            _ => {}
        }
    }
    drop(live);
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(connected, "Socket.IO namespace handshake failed");
    assert!(received, "No live message arrived");
    let requests: Vec<_> = requests_rx.try_iter().collect();
    assert!(!requests.is_empty());
    for (path, authenticated) in requests {
        assert_eq!(path, "/prefix/socket.io/");
        assert!(
            authenticated,
            "Socket authentication was missing or incorrectly encoded"
        );
    }
}
