use bluebubbles_linux::{
    api::{Api, ApiResult},
    api_actions::{contact_names, normalize_address, ServerInfo},
    cache::{Cache, OpenCache},
    firebase::FirebaseConfig,
    model::{merge_messages, Chat, Message},
    realtime::{LiveConnection, LiveEvent},
};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

pub enum Event {
    FirebaseNotice(String),
    PushSetup(ApiResult<()>),
    FirebaseConfig {
        server: String,
        result: ApiResult<FirebaseConfig>,
    },
    FirebaseResolved {
        server: String,
        config: FirebaseConfig,
        result: ApiResult<String>,
    },
    Contacts(HashMap<String, String>),
    ServerInfo(ServerInfo),
    Live(LiveEvent),
    CacheReady(Box<OpenCache>),
    Connected(Vec<Chat>),
    Messages {
        chat: String,
        messages: Vec<Message>,
        older: bool,
    },
    Refreshed {
        chats: Vec<Chat>,
        messages: Option<(String, Vec<Message>)>,
    },
    Sent {
        chat: String,
        draft: String,
    },
    Created(Box<Chat>),
    Notice(String),
    Cancelled,
}

#[derive(Clone)]
enum Work {
    Firebase,
    Foreground,
    Refresh(Option<String>),
    Messages(String),
    Background,
}

type Completion = (u64, Work, ApiResult<Event>);

pub struct App {
    pub push_previews: bool,
    pub push_at_login: bool,
    pub firebase_open: bool,
    pub firebase_busy: bool,
    pub firebase_status: String,
    pub firebase_configs: HashMap<String, FirebaseConfig>,
    pub firebase_auto: bool,
    next_firebase: Instant,
    pub server: String,
    pub password: String,
    pub dark: bool,
    pub api: Option<Api>,
    pub connected: bool,
    pub busy: bool,
    pub refreshing: bool,
    pub socket_online: bool,
    pub persist_history: bool,
    pub typing: HashMap<String, Instant>,
    pub contacts: HashMap<String, String>,
    pub server_info: Option<ServerInfo>,
    live: Option<LiveConnection>,
    cache: Option<Cache>,
    cache_errors: Option<Receiver<String>>,
    pub status: String,
    pub error: Option<String>,
    pub chats: Vec<Chat>,
    pub selected: Option<String>,
    pub messages: HashMap<String, Vec<Message>>,
    pub drafts: HashMap<String, String>,
    edited_drafts: HashSet<String>,
    pub search: String,
    pub message_search: String,
    pub more_messages: HashMap<String, bool>,
    pub new_chat: bool,
    pub recipients: String,
    pub initial_message: String,
    pub scroll_to_bottom: bool,
    pub last_sync: Option<Instant>,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    pending_messages: HashSet<String>,
    refresh_requested: bool,
    generation: u64,
    next_refresh: Instant,
    display: crate::display::DisplayScaling,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let server = cc
            .storage
            .and_then(|s| s.get_string("server"))
            .unwrap_or_default();
        let dark = cc.storage.and_then(|s| s.get_string("dark")).as_deref() != Some("false");
        crate::theme::apply(&cc.egui_ctx, dark);
        let mut app = Self::with_settings(server, dark);
        app.persist_history = cc
            .storage
            .and_then(|s| s.get_string("persist_history"))
            .as_deref()
            == Some("true");
        app.firebase_configs = cc
            .storage
            .and_then(|s| s.get_string("firebase_configs"))
            .and_then(|s| serde_json::from_str::<HashMap<String, FirebaseConfig>>(&s).ok())
            .unwrap_or_default();
        app.firebase_configs
            .retain(|_, config| config.endpoint().is_ok());
        app.firebase_auto = cc
            .storage
            .and_then(|s| s.get_string("firebase_auto"))
            .as_deref()
            != Some("false");
        app
    }

    fn with_settings(server: String, dark: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            push_previews: false,
            push_at_login: false,
            firebase_open: false,
            firebase_busy: false,
            firebase_status:
                "Connect to load Firebase configuration, or import google-services.json.".into(),
            firebase_configs: HashMap::new(),
            firebase_auto: true,
            next_firebase: Instant::now(),
            server,
            password: String::new(),
            dark,
            api: None,
            connected: false,
            busy: false,
            refreshing: false,
            socket_online: false,
            persist_history: false,
            typing: HashMap::new(),
            contacts: HashMap::new(),
            server_info: None,
            live: None,
            cache: None,
            cache_errors: None,
            status: "Connect to your Mac to get started".into(),
            error: None,
            chats: Vec::new(),
            selected: None,
            messages: HashMap::new(),
            drafts: HashMap::new(),
            edited_drafts: HashSet::new(),
            search: String::new(),
            message_search: String::new(),
            more_messages: HashMap::new(),
            new_chat: false,
            recipients: String::new(),
            initial_message: String::new(),
            scroll_to_bottom: false,
            last_sync: None,
            tx,
            rx,
            pending_messages: HashSet::new(),
            refresh_requested: false,
            generation: 0,
            next_refresh: Instant::now(),
            display: crate::display::DisplayScaling::default(),
        }
    }

    pub fn run(
        &mut self,
        ctx: &egui::Context,
        status: &str,
        job: impl FnOnce() -> ApiResult<Event> + Send + 'static,
    ) {
        self.start(ctx, Work::Foreground, status, job);
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        work: Work,
        status: &str,
        job: impl FnOnce() -> ApiResult<Event> + Send + 'static,
    ) {
        match &work {
            Work::Firebase => {
                if self.firebase_busy {
                    return;
                }
                self.firebase_busy = true;
            }
            Work::Foreground => {
                if self.busy {
                    return;
                }
                self.busy = true;
                self.status = status.into();
            }
            Work::Refresh(chat) => {
                if self.refreshing {
                    return;
                }
                self.refreshing = true;
                if let Some(chat) = chat {
                    self.pending_messages.insert(chat.clone());
                }
            }
            Work::Messages(chat) => {
                if !self.pending_messages.insert(chat.clone()) {
                    return;
                }
            }
            Work::Background => {}
        }
        let tx = self.tx.clone();
        let generation = self.generation;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = job();
            let _ = tx.send((generation, work, result));
            ctx.request_repaint();
        });
    }

    pub fn connect(&mut self, ctx: &egui::Context) {
        match Api::new(&self.server, &self.password) {
            Ok(api) => {
                self.api = Some(api.clone());
                self.error = None;
                self.run(ctx, "Connecting…", move || {
                    api.connect()?;
                    Ok(Event::Connected(api.all_chats()?))
                });
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn firebase_key(&self) -> String {
        Api::new(&self.server, "validation-only")
            .map(|api| api.server_identity())
            .unwrap_or_default()
    }

    pub fn enable_push(&mut self, ctx: &egui::Context) {
        if let (Some(api), Some(config)) = (
            self.api.clone(),
            self.firebase_configs.get(&self.firebase_key()).cloned(),
        ) {
            let preferences = bluebubbles_linux::push::Preferences {
                previews: self.push_previews,
            };
            let at_login = self.push_at_login;
            self.firebase_status = "Registering for background notifications…".into();
            self.start(ctx, Work::Firebase, "", move || {
                Ok(Event::PushSetup(bluebubbles_linux::push::enable(
                    api,
                    config,
                    preferences,
                    at_login,
                )))
            });
        }
    }

    pub fn disable_push(&mut self, ctx: &egui::Context) {
        if let Some(config) = self.firebase_configs.get(&self.firebase_key()).cloned() {
            self.start(ctx, Work::Firebase, "", move || {
                bluebubbles_linux::push::disable(&config)?;
                Ok(Event::FirebaseNotice(
                    "Background notifications disabled.".into(),
                ))
            });
        }
    }

    pub fn test_notification(&mut self, ctx: &egui::Context) {
        self.start(ctx, Work::Firebase, "", move || {
            bluebubbles_linux::push::test_notification()?;
            Ok(Event::FirebaseNotice(
                "Test notification sent to your desktop.".into(),
            ))
        });
    }

    pub fn reset_push(&mut self, ctx: &egui::Context) {
        if let Some(config) = self.firebase_configs.get(&self.firebase_key()).cloned() {
            self.start(ctx, Work::Firebase, "", move || {
                bluebubbles_linux::push::reset(&config)?;
                Ok(Event::FirebaseNotice(
                    "Registration reset. Enable notifications to register again.".into(),
                ))
            });
        }
    }

    pub fn load_firebase(&mut self, ctx: &egui::Context) {
        if let Some(api) = self.api.clone() {
            let server = self.firebase_key();
            self.start(ctx, Work::Firebase, "", move || {
                Ok(Event::FirebaseConfig {
                    server,
                    result: api.firebase_config(),
                })
            });
        }
    }

    pub fn import_firebase(&mut self, ctx: &egui::Context) {
        let server = self.firebase_key();
        self.start(ctx, Work::Firebase, "", move || {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Import google-services.json")
                .add_filter("JSON", &["json"])
                .pick_file()
            else {
                return Ok(Event::Cancelled);
            };
            Ok(Event::FirebaseConfig {
                server,
                result: FirebaseConfig::import(&path),
            })
        });
    }

    pub fn resolve_firebase(&mut self, ctx: &egui::Context) {
        if self.firebase_busy || self.busy {
            return;
        }
        let server = self.firebase_key();
        let Some(config) = self.firebase_configs.get(&server).cloned() else {
            return;
        };
        self.next_firebase = Instant::now() + Duration::from_secs(60);
        self.firebase_status = "Looking up the current server URL…".into();
        let api = self
            .api
            .clone()
            .or_else(|| Api::new(&self.server, &self.password).ok());
        self.start(ctx, Work::Firebase, "", move || {
            let result = config.discover().and_then(|address| {
                if let Some(api) = api {
                    api.at_server(&address)?.info()?;
                }
                Ok(address)
            });
            Ok(Event::FirebaseResolved {
                server,
                config,
                result,
            })
        });
    }

    pub fn disconnect(&mut self) {
        self.generation += 1;
        self.firebase_busy = false;
        self.api = None;
        self.connected = false;
        self.busy = false;
        self.refreshing = false;
        self.refresh_requested = false;
        self.pending_messages.clear();
        self.live = None;
        self.cache = None;
        self.cache_errors = None;
        self.socket_online = false;
        self.typing.clear();
        self.contacts.clear();
        self.server_info = None;
        self.password.clear();
        self.chats.clear();
        self.messages.clear();
        self.more_messages.clear();
        self.drafts.clear();
        self.edited_drafts.clear();
        self.selected = None;
        self.last_sync = None;
        self.error = None;
        self.new_chat = false;
        self.recipients.clear();
        self.initial_message.clear();
        self.status = "Disconnected".into();
    }

    pub fn select(&mut self, ctx: &egui::Context, guid: String) {
        self.selected = Some(guid.clone());
        self.message_search.clear();
        self.scroll_to_bottom = true;
        if let Some(api) = self.api.clone() {
            let known = self.known_messages(&guid);
            self.start(ctx, Work::Messages(guid.clone()), "", move || {
                Ok(Event::Messages {
                    messages: api.refresh_messages(&guid, &known)?,
                    chat: guid,
                    older: false,
                })
            });
        }
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.refreshing {
            self.refresh_requested = true;
            return;
        }
        if let Some(api) = self.api.clone() {
            let selected = self
                .selected
                .clone()
                .filter(|chat| !self.pending_messages.contains(chat));
            let known = selected
                .as_ref()
                .map(|guid| self.known_messages(guid))
                .unwrap_or_default();
            self.start(ctx, Work::Refresh(selected.clone()), "", move || {
                let chats = api.all_chats()?;
                let messages = match selected {
                    Some(guid) => Some((guid.clone(), api.refresh_messages(&guid, &known)?)),
                    None => None,
                };
                Ok(Event::Refreshed { chats, messages })
            });
        }
    }

    pub fn load_older(&mut self, ctx: &egui::Context) {
        if let (Some(api), Some(chat)) = (self.api.clone(), self.selected.clone()) {
            let before = self
                .messages
                .get(&chat)
                .and_then(|m| m.last())
                .and_then(|m| m.date_created)
                .map(|t| t.saturating_add(1));
            let offset = self.messages.get(&chat).map_or(0, Vec::len);
            self.start(ctx, Work::Messages(chat.clone()), "", move || {
                Ok(Event::Messages {
                    messages: api.messages_page(&chat, before, offset)?,
                    chat,
                    older: true,
                })
            });
        }
    }

    fn known_messages(&self, chat: &str) -> HashSet<String> {
        self.messages
            .get(chat)
            .into_iter()
            .flatten()
            .map(|m| m.guid.clone())
            .collect()
    }

    pub fn messages_loading(&self, chat: &str) -> bool {
        self.pending_messages.contains(chat)
    }

    pub fn save_draft(&mut self, chat: &str) {
        self.edited_drafts.insert(chat.into());
        if let Some(cache) = &self.cache {
            cache.draft(
                chat.into(),
                self.drafts.get(chat).cloned().unwrap_or_default(),
            );
        }
    }

    pub fn chat_title(&self, chat: &Chat) -> String {
        if chat
            .display_name
            .as_ref()
            .is_some_and(|name| !name.trim().is_empty())
            || chat.participants.is_empty()
        {
            return chat.title();
        }
        chat.participants
            .iter()
            .map(|handle| {
                self.contacts
                    .get(&normalize_address(&handle.address))
                    .cloned()
                    .unwrap_or_else(|| handle.address.clone())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn start_session_services(&mut self, ctx: &egui::Context) {
        self.load_firebase(ctx);
        if let Some(api) = self.api.clone() {
            let contacts_api = api.clone();
            self.start(ctx, Work::Background, "", move || {
                contacts_api
                    .contacts()
                    .map(|values| Event::Contacts(contact_names(&values)))
            });
            let info_api = api.clone();
            self.start(ctx, Work::Background, "", move || {
                info_api.info().map(Event::ServerInfo)
            });
            let generation = self.generation;
            let tx = self.tx.clone();
            let repaint = ctx.clone();
            self.live = Some(LiveConnection::start(api.clone(), move |event| {
                let _ = tx.send((generation, Work::Background, Ok(Event::Live(event))));
                repaint.request_repaint();
            }));
            if self.persist_history && self.cache.is_none() {
                self.start(ctx, Work::Background, "", move || {
                    Cache::open(&api.server_identity())
                        .map(|cache| Event::CacheReady(Box::new(cache)))
                });
            }
        }
    }

    fn receive_live(&mut self, event: LiveEvent) {
        match event {
            LiveEvent::Connected => {
                self.socket_online = true;
                self.next_refresh = Instant::now();
            }
            LiveEvent::Disconnected => {
                self.socket_online = false;
                self.next_refresh = Instant::now();
            }
            LiveEvent::Data { name, data } => match name.as_str() {
                "typing-indicator" => {
                    if let Some(chat) = data.get("guid").and_then(|v| v.as_str()) {
                        if data.get("display").and_then(|v| v.as_bool()) == Some(true) {
                            self.typing.insert(chat.into(), Instant::now());
                        } else {
                            self.typing.remove(chat);
                        }
                    }
                }
                "new-message" | "updated-message" => {
                    let parsed = serde_json::from_value::<Message>(data.clone());
                    let chats = data.get("chats").and_then(|v| v.as_array());
                    if let (Ok(message), Some(chats)) = (parsed, chats) {
                        for value in chats {
                            if let Ok(mut chat) = serde_json::from_value::<Chat>(value.clone()) {
                                let guid = chat.guid.clone();
                                self.typing.remove(&guid);
                                let messages = self.messages.entry(guid.clone()).or_default();
                                merge_messages(messages, vec![message.clone()]);
                                if let Some(cache) = &self.cache {
                                    cache.messages(guid.clone(), messages.clone());
                                }
                                // Update the preview only when this is the newest message.
                                let old = self.chats.iter().find(|c| c.guid == guid);
                                if message.activity_timestamp() >= old.and_then(Chat::last_activity)
                                {
                                    chat.last_message = Some(message.clone());
                                    if let Some(old) = old {
                                        chat.display_name = old.display_name.clone();
                                        if chat.participants.is_empty() {
                                            chat.participants = old.participants.clone();
                                        }
                                    }
                                    self.merge_chats(vec![chat]);
                                }
                            }
                        }
                    } else {
                        self.next_refresh = Instant::now();
                    }
                }
                _ => {
                    self.next_refresh = Instant::now();
                }
            },
        }
    }

    pub fn send(&mut self, ctx: &egui::Context) {
        if let (Some(api), Some(chat)) = (self.api.clone(), self.selected.clone()) {
            let draft = self.drafts.get(&chat).cloned().unwrap_or_default();
            if draft.trim().is_empty() {
                return;
            }
            let temp_guid = uuid::Uuid::new_v4().to_string();
            self.error = None;
            self.run(ctx, "Sending…", move || {
                api.send_text(&chat, &draft, &temp_guid)
                    .map_err(|e| format!("{e} Your draft was kept. Check the conversation before retrying; the server may have received it."))?;
                Ok(Event::Sent { chat, draft })
            });
        }
    }

    pub fn attach(&mut self, ctx: &egui::Context) {
        if let (Some(api), Some(chat)) = (self.api.clone(), self.selected.clone()) {
            self.run(ctx, "Choosing / sending attachment…", move || {
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Send attachment")
                    .pick_file()
                else {
                    return Ok(Event::Cancelled);
                };
                api.send_attachment(&chat, &path, &uuid::Uuid::new_v4().to_string())
                    .map_err(|e| {
                        format!("{e} Check the conversation before sending the attachment again.")
                    })?;
                Ok(Event::Notice("Attachment sent".into()))
            });
        }
    }

    pub fn download(&mut self, ctx: &egui::Context, guid: String, filename: String) {
        if let Some(api) = self.api.clone() {
            self.run(ctx, "Saving attachment…", move || {
                // Server-provided names are suggestions, never filesystem paths.
                let name = std::path::Path::new(&filename)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment");
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Save attachment (choose a new filename)")
                    .set_file_name(name)
                    .save_file()
                else {
                    return Ok(Event::Cancelled);
                };
                api.download(&guid, &path)?;
                Ok(Event::Notice(format!("Saved {}", path.display())))
            });
        }
    }

    pub fn create_chat(&mut self, ctx: &egui::Context) {
        if let Some(api) = self.api.clone() {
            let addresses: Vec<String> = self
                .recipients
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if addresses.is_empty() || self.initial_message.trim().is_empty() {
                return;
            }
            let message = self.initial_message.clone();
            self.error = None;
            self.run(ctx, "Creating conversation…", move || {
                api.create_chat(addresses, &message).map(|chat| Event::Created(Box::new(chat)))
                    .map_err(|e| format!("{e} Check the chat list before retrying; your initial message may have been sent."))
            });
        }
    }

    fn merge_chats(&mut self, chats: Vec<Chat>) {
        for chat in chats {
            if let Some(old) = self.chats.iter_mut().find(|c| c.guid == chat.guid) {
                let mut incoming = chat;
                // An HTTP snapshot may have started before a newer socket event.
                if let Some(previous) = &old.last_message {
                    match &incoming.last_message {
                        Some(next) if next.guid == previous.guid => {
                            let mut merged = vec![previous.clone()];
                            merge_messages(&mut merged, vec![next.clone()]);
                            incoming.last_message = merged.pop();
                        }
                        next if next.as_ref().and_then(Message::activity_timestamp)
                            < previous.activity_timestamp() =>
                        {
                            incoming.last_message = Some(previous.clone());
                        }
                        _ => {}
                    }
                }
                *old = incoming;
            } else {
                self.chats.push(chat);
            }
        }
        self.chats.sort_by(|a, b| {
            b.last_activity()
                .cmp(&a.last_activity())
                .then(a.guid.cmp(&b.guid))
        });
        if let Some(cache) = &self.cache {
            cache.chats(self.chats.clone());
        }
    }

    fn refresh_chat_preview(&mut self, guid: &str) {
        let latest = self
            .messages
            .get(guid)
            .and_then(|history| {
                history
                    .iter()
                    .max_by_key(|message| message.activity_timestamp())
            })
            .cloned();
        if let (Some(mut chat), Some(message)) = (
            self.chats.iter().find(|chat| chat.guid == guid).cloned(),
            latest,
        ) {
            chat.last_message = Some(message);
            // merge_chats keeps a newer preview if history is an older page.
            self.merge_chats(vec![chat]);
        }
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((generation, work, result)) = self.rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            match &work {
                Work::Firebase => self.firebase_busy = false,
                Work::Foreground => {
                    self.busy = false;
                    self.status = "Connected".into();
                }
                Work::Refresh(chat) => {
                    self.refreshing = false;
                    if let Some(chat) = chat {
                        self.pending_messages.remove(chat);
                    }
                    self.next_refresh = Instant::now()
                        + if result.is_ok() {
                            Duration::from_secs(if self.socket_online { 60 } else { 3 })
                        } else {
                            Duration::from_secs(10)
                        };
                    if std::mem::take(&mut self.refresh_requested) {
                        self.next_refresh = Instant::now();
                    }
                }
                Work::Messages(chat) => {
                    self.pending_messages.remove(chat);
                }
                Work::Background => {}
            }
            let event = match result {
                Ok(event) => event,
                Err(error) => {
                    if matches!(work, Work::Firebase) {
                        self.firebase_status = error;
                        continue;
                    }
                    if matches!(&work, Work::Messages(chat) if self.selected.as_ref() != Some(chat))
                    {
                        continue;
                    }
                    self.error = Some(error);
                    if matches!(work, Work::Foreground) {
                        self.status = "Request failed".into();
                    }
                    if self.firebase_auto
                        && matches!(work, Work::Refresh(_) | Work::Foreground)
                        && self.error.as_deref().is_some_and(|error| {
                            [
                                "Cannot reach the server",
                                "Request timed out",
                                "Network request failed",
                                "Cloudflare",
                                "Server returned HTTP 530",
                                "Server returned HTTP 502",
                                "Server returned HTTP 503",
                            ]
                            .iter()
                            .any(|prefix| error.starts_with(prefix))
                        })
                        && Instant::now() >= self.next_firebase
                    {
                        self.resolve_firebase(ctx);
                    }
                    continue;
                }
            };
            match event {
                Event::FirebaseNotice(notice) => self.firebase_status = notice,
                Event::PushSetup(result) => self.firebase_status = match result {
                    Ok(()) => {
                        "Notifications enabled. The receiver keeps running when this window closes."
                            .into()
                    }
                    Err(error) => error,
                },
                Event::FirebaseConfig { server, result } => match result {
                    Ok(config) => {
                        self.firebase_status = format!("Configured: {}", config.project_id);
                        self.firebase_configs.insert(server, config);
                    }
                    Err(error) => {
                        self.firebase_status =
                            format!("Firebase configuration unavailable: {error}")
                    }
                },
                Event::FirebaseResolved {
                    server,
                    config,
                    result,
                } => {
                    if server != self.firebase_key() {
                        continue;
                    }
                    match result {
                        Ok(address) => {
                            if self.busy {
                                self.firebase_status = "URL found. Waiting for the current operation; check again shortly.".into();
                                continue;
                            }
                            let old = self.firebase_key();
                            let next = match self.api.as_ref() {
                                Some(api) => api.at_server(&address),
                                None => Api::new(&address, &self.password),
                            };
                            self.server = address;
                            self.firebase_configs.insert(self.firebase_key(), config);
                            self.firebase_status =
                                "Current server URL loaded from Firebase.".into();
                            if old != self.firebase_key() {
                                self.generation += 1;
                                self.live = None;
                                self.socket_online = false;
                                self.refreshing = false;
                                self.pending_messages.clear();
                                self.refresh_requested = false;
                                if let Ok(api) = next {
                                    self.api = Some(api.clone());
                                    self.run(
                                        ctx,
                                        "Reconnecting to recovered server…",
                                        move || {
                                            api.connect()?;
                                            Ok(Event::Connected(api.all_chats()?))
                                        },
                                    );
                                }
                            }
                        }
                        Err(error) => self.firebase_status = error,
                    }
                }
                Event::Contacts(contacts) => self.contacts = contacts,
                Event::ServerInfo(info) => self.server_info = Some(info),
                Event::Live(event) => self.receive_live(event),
                Event::CacheReady(ready) => {
                    let OpenCache {
                        cache,
                        snapshot,
                        errors,
                    } = *ready;
                    // Cached rows fill gaps only; never replace newer network data.
                    let missing = snapshot
                        .chats
                        .into_iter()
                        .filter(|c| !self.chats.iter().any(|old| old.guid == c.guid))
                        .collect();
                    self.merge_chats(missing);
                    for (chat, history) in snapshot.messages {
                        let current = self.messages.entry(chat).or_default();
                        let missing = history
                            .into_iter()
                            .filter(|m| !current.iter().any(|old| old.guid == m.guid))
                            .collect();
                        merge_messages(current, missing);
                    }
                    for (chat, draft) in snapshot.drafts {
                        if !self.edited_drafts.contains(&chat)
                            && self.drafts.get(&chat).is_none_or(|s| s.is_empty())
                        {
                            self.drafts.insert(chat, draft);
                        }
                    }
                    cache.chats(self.chats.clone());
                    for (chat, messages) in &self.messages {
                        cache.messages(chat.clone(), messages.clone());
                    }
                    for (chat, draft) in &self.drafts {
                        cache.draft(chat.clone(), draft.clone());
                    }
                    self.cache = Some(cache);
                    self.cache_errors = Some(errors);
                }
                Event::Connected(chats) => {
                    self.connected = true;
                    self.error = None;
                    self.password.clear();
                    self.merge_chats(chats);
                    self.last_sync = Some(Instant::now());
                    self.start_session_services(ctx);
                    if let Some(guid) = self
                        .selected
                        .clone()
                        .or_else(|| self.chats.first().map(|chat| chat.guid.clone()))
                    {
                        self.select(ctx, guid);
                    }
                }
                Event::Messages {
                    chat,
                    messages,
                    older,
                } => {
                    if older || !self.more_messages.contains_key(&chat) {
                        self.more_messages
                            .insert(chat.clone(), messages.len() == 100);
                    }
                    if self.selected.as_ref() == Some(&chat) && !older {
                        self.scroll_to_bottom = true;
                    }
                    let history = self.messages.entry(chat.clone()).or_default();
                    merge_messages(history, messages);
                    if let Some(cache) = &self.cache {
                        cache.messages(chat.clone(), history.clone());
                    }
                    self.refresh_chat_preview(&chat);
                    self.last_sync = Some(Instant::now());
                }
                Event::Refreshed { chats, messages } => {
                    self.merge_chats(chats);
                    if let Some((chat, messages)) = messages {
                        self.more_messages
                            .entry(chat.clone())
                            .or_insert(messages.len() >= 100);
                        let history = self.messages.entry(chat.clone()).or_default();
                        merge_messages(history, messages);
                        if let Some(cache) = &self.cache {
                            cache.messages(chat.clone(), history.clone());
                        }
                        self.refresh_chat_preview(&chat);
                    }
                    self.last_sync = Some(Instant::now());
                }
                Event::Sent { chat, draft } => {
                    if self.drafts.get(&chat) == Some(&draft) {
                        self.drafts.remove(&chat);
                        self.save_draft(&chat);
                    }
                    if self.selected.as_ref() == Some(&chat) {
                        self.scroll_to_bottom = true;
                    }
                    self.refresh(ctx);
                }
                Event::Created(chat) => {
                    let guid = chat.guid.clone();
                    self.merge_chats(vec![*chat]);
                    self.new_chat = false;
                    self.recipients.clear();
                    self.initial_message.clear();
                    self.select(ctx, guid);
                }
                Event::Notice(notice) => {
                    self.status = notice;
                    self.next_refresh = Instant::now();
                }
                Event::Cancelled => {}
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.display.update(ctx);
        self.receive(ctx);
        if let Some(errors) = &self.cache_errors {
            while let Ok(error) = errors.try_recv() {
                self.error = Some(error);
            }
        }
        self.typing
            .retain(|_, time| time.elapsed() < Duration::from_secs(10));
        if self.connected && !self.busy && !self.refreshing && Instant::now() >= self.next_refresh {
            self.refresh(ctx);
        }
        crate::views::show(self, ctx);
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Do not persist credentials accidentally pasted into an invalid URL.
        let server = if Api::new(&self.server, "validation-only").is_ok() {
            self.server.clone()
        } else {
            String::new()
        };
        storage.set_string("server", server);
        storage.set_string("dark", self.dark.to_string());
        storage.set_string("persist_history", self.persist_history.to_string());
        storage.set_string("firebase_auto", self.firebase_auto.to_string());
        if let Ok(configs) = serde_json::to_string(&self.firebase_configs) {
            storage.set_string("firebase_configs", configs);
        }
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.live = None;
        if let Some(cache) = &self.cache {
            let _ = cache.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_activity_uses_history_and_never_promotes_receipts_or_older_pages() {
        let mut app = App::with_settings(String::new(), true);
        let chat = |guid: &str, time: i64| {
            serde_json::from_value(serde_json::json!({"guid":guid,"lastMessage":{"guid":format!("{guid}-message"),"dateCreated":time}})).unwrap()
        };
        app.merge_chats(vec![chat("older", 10), chat("newer", 20)]);
        assert_eq!(app.chats[0].guid, "newer");
        app.messages.insert(
            "older".into(),
            vec![Message {
                guid: "latest".into(),
                date_created: Some(30),
                ..Default::default()
            }],
        );
        app.refresh_chat_preview("older");
        assert_eq!(app.chats[0].guid, "older");
        app.merge_chats(vec![chat("older", 10)]);
        assert_eq!(app.chats[0].last_activity(), Some(30));
        app.messages.insert(
            "newer".into(),
            vec![Message {
                guid: "old-read-message".into(),
                date_created: Some(1),
                date_read: Some(999),
                ..Default::default()
            }],
        );
        app.refresh_chat_preview("newer");
        assert_eq!(app.chats[0].guid, "older");
        assert_eq!(app.chats[1].last_activity(), Some(20));
        app.merge_chats(vec![serde_json::from_value(serde_json::json!({"guid":"fallback", "lastMessage":{"guid":"fallback-message","dateCreated":0,"dateDelivered":40}})).unwrap()]);
        assert_eq!(app.chats[0].guid, "fallback");
    }

    #[test]
    fn firebase_lookup_does_not_interrupt_sending_or_switch_another_account() {
        let mut app = App::with_settings("https://old.example".into(), true);
        app.busy = true;
        app.status = "Sending…".into();
        let config = FirebaseConfig {
            project_id: "bb-example".into(),
            database_url: None,
        };
        app.tx
            .send((
                app.generation,
                Work::Firebase,
                Ok(Event::FirebaseResolved {
                    server: app.firebase_key(),
                    config: config.clone(),
                    result: Ok("https://new.example".into()),
                }),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert!(app.busy);
        assert_eq!(app.server, "https://old.example");
        assert_eq!(app.status, "Sending…");
        app.busy = false;
        app.server = "https://another.example".into();
        app.tx
            .send((
                app.generation,
                Work::Firebase,
                Ok(Event::FirebaseResolved {
                    server: "https://old.example/".into(),
                    config,
                    result: Ok("https://new.example".into()),
                }),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(app.server, "https://another.example");
    }

    #[test]
    fn firebase_failure_preserves_the_working_server_and_drafts() {
        let mut app = App::with_settings("https://old.example".into(), true);
        app.drafts.insert("chat".into(), "Unsent".into());
        app.tx
            .send((
                app.generation,
                Work::Firebase,
                Ok(Event::FirebaseResolved {
                    server: app.firebase_key(),
                    config: FirebaseConfig {
                        project_id: "bb-example".into(),
                        database_url: None,
                    },
                    result: Err("Firebase unavailable".into()),
                }),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(app.server, "https://old.example");
        assert_eq!(app.drafts["chat"], "Unsent");
        assert_eq!(app.firebase_status, "Firebase unavailable");
    }

    #[test]
    fn live_message_survives_a_stale_poll_and_does_not_release_send() {
        let mut app = App::with_settings(String::new(), true);
        app.busy = true;
        app.status = "Sending…".into();
        app.selected = Some("other".into());
        app.receive_live(LiveEvent::Data {
            name: "new-message".into(),
            data: serde_json::json!({"guid":"new", "text":"Newest", "dateCreated":20,
                "dateRead":30,"chats":[{"guid":"chat","displayName":"Name"}]}),
        });
        app.merge_chats(vec![serde_json::from_value(serde_json::json!({
            "guid":"chat", "lastMessage":{"guid":"old","text":"Stale","dateCreated":10}
        }))
        .unwrap()]);
        assert_eq!(app.chats[0].last_message.as_ref().unwrap().guid, "new");
        assert_eq!(app.messages["chat"][0].text.as_deref(), Some("Newest"));
        assert_eq!(app.selected.as_deref(), Some("other"));
        assert!(app.busy);
        assert_eq!(app.status, "Sending…");
    }

    #[test]
    fn stale_socket_events_after_disconnect_are_ignored() {
        let mut app = App::with_settings(String::new(), true);
        let generation = app.generation;
        app.disconnect();
        app.tx
            .send((
                generation,
                Work::Background,
                Ok(Event::Live(LiveEvent::Connected)),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert!(!app.socket_online);
    }

    fn receive_one(app: &mut App, ctx: &egui::Context) {
        let completion = app.rx.recv_timeout(Duration::from_secs(3)).unwrap();
        app.tx.send(completion).unwrap();
        app.receive(ctx);
    }

    #[test]
    fn paused_poll_allows_chat_switch_and_send_without_clearing_send_busy_state() {
        let mut app = App::with_settings(String::new(), true);
        let ctx = egui::Context::default();
        app.selected = Some("old".into());
        let (release_poll, wait_poll) = mpsc::channel();
        app.start(&ctx, Work::Refresh(Some("old".into())), "", move || {
            wait_poll.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(Event::Refreshed {
                chats: vec![],
                messages: Some((
                    "old".into(),
                    vec![Message {
                        guid: "old-message".into(),
                        ..Default::default()
                    }],
                )),
            })
        });
        assert!(app.refreshing);
        assert!(!app.busy);
        app.select(&ctx, "new".into());
        assert_eq!(app.selected.as_deref(), Some("new"));
        let (release_send, wait_send) = mpsc::channel();
        app.run(&ctx, "Sending…", move || {
            wait_send.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(Event::Cancelled)
        });
        assert!(app.busy);
        release_poll.send(()).unwrap();
        receive_one(&mut app, &ctx);
        assert!(app.busy, "Poll completion must not enable a duplicate send");
        assert_eq!(app.status, "Sending…");
        assert_eq!(app.selected.as_deref(), Some("new"));
        assert_eq!(app.messages["old"][0].guid, "old-message");
        assert!(!app.messages.contains_key("new"));
        release_send.send(()).unwrap();
        receive_one(&mut app, &ctx);
        assert!(!app.busy);
    }

    #[test]
    fn late_chat_load_does_not_scroll_the_new_conversation() {
        let mut app = App::with_settings(String::new(), true);
        app.selected = Some("new".into());
        app.scroll_to_bottom = false;
        app.tx
            .send((
                app.generation,
                Work::Messages("old".into()),
                Ok(Event::Messages {
                    chat: "old".into(),
                    messages: vec![Message {
                        guid: "old-message".into(),
                        ..Default::default()
                    }],
                    older: false,
                }),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(app.selected.as_deref(), Some("new"));
        assert!(!app.scroll_to_bottom);
        assert_eq!(app.messages["old"][0].guid, "old-message");
    }

    #[test]
    fn repeated_chat_clicks_share_one_pending_request() {
        let mut app = App::with_settings(String::new(), true);
        let ctx = egui::Context::default();
        let (release, wait) = mpsc::channel();
        app.start(&ctx, Work::Messages("chat".into()), "", move || {
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(Event::Cancelled)
        });
        let (duplicate_tx, duplicate_rx) = mpsc::channel();
        app.start(&ctx, Work::Messages("chat".into()), "", move || {
            duplicate_tx.send(()).unwrap();
            Ok(Event::Cancelled)
        });
        assert!(matches!(
            duplicate_rx.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert!(app.messages_loading("chat"));
        assert!(!app.busy);
        release.send(()).unwrap();
        receive_one(&mut app, &ctx);
        assert!(!app.messages_loading("chat"));
    }

    #[test]
    fn refresh_requested_during_a_poll_runs_after_it_finishes() {
        let mut app = App::with_settings(String::new(), true);
        app.refreshing = true;
        app.refresh(&egui::Context::default());
        app.tx
            .send((
                app.generation,
                Work::Refresh(None),
                Ok(Event::Refreshed {
                    chats: vec![],
                    messages: None,
                }),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert!(!app.refreshing);
        assert!(app.next_refresh <= Instant::now());
    }

    #[test]
    fn disconnect_ignores_late_connection_results() {
        let mut app = App::with_settings(String::new(), true);
        let old_generation = app.generation;
        app.disconnect();
        app.tx
            .send((
                old_generation,
                Work::Foreground,
                Ok(Event::Connected(Vec::new())),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert!(!app.connected);
        assert_eq!(app.status, "Disconnected");
    }

    #[test]
    fn send_acknowledgement_preserves_new_typing() {
        let mut app = App::with_settings(String::new(), true);
        app.drafts.insert("chat".into(), "next message".into());
        app.tx
            .send((
                app.generation,
                Work::Foreground,
                Ok(Event::Sent {
                    chat: "chat".into(),
                    draft: "sent message".into(),
                }),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(app.drafts.get("chat").unwrap(), "next message");
    }

    #[test]
    fn failed_send_keeps_draft_and_releases_busy_state() {
        let mut app = App::with_settings(String::new(), true);
        app.busy = true;
        app.drafts.insert("chat".into(), "keep me".into());
        app.tx
            .send((
                app.generation,
                Work::Foreground,
                Err("Network failure".into()),
            ))
            .unwrap();
        app.receive(&egui::Context::default());
        assert_eq!(app.drafts.get("chat").unwrap(), "keep me");
        assert!(!app.busy);
        assert_eq!(app.error.as_deref(), Some("Network failure"));
    }
}
