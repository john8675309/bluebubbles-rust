//! Linux StatusNotifierItem integration; D-Bus work stays off the UI thread.
use eframe::egui;
use ksni::blocking::TrayMethods;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Show,
    Quit,
}

pub struct Tray {
    online: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    rx: mpsc::Receiver<Action>,
}

#[derive(Clone)]
struct Item {
    online: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    tx: mpsc::Sender<Action>,
    ctx: egui::Context,
    icon: Vec<u8>,
    icon_file: String,
}

impl Item {
    fn send(&self, action: Action) {
        if matches!(action, Action::Quit) {
            self.quit.store(true, Ordering::Relaxed);
        }
        let _ = self.tx.send(action);
        // Requesting visibility directly also wakes a hidden window on Wayland.
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.ctx.request_repaint();
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "app.bluebubbles.RustLinux".into()
    }
    fn title(&self) -> String {
        "BlueBubbles".into()
    }
    fn icon_name(&self) -> String {
        self.icon_file.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "BlueBubbles".into(),
            description: "Click to open BlueBubbles".into(),
            ..Default::default()
        }
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![ksni::Icon {
            width: 32,
            height: 32,
            data: self.icon.clone(),
        }]
    }
    fn activate(&mut self, _: i32, _: i32) {
        self.send(Action::Show);
    }
    fn secondary_activate(&mut self, _: i32, _: i32) {
        self.send(Action::Show);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: "Open BlueBubbles".into(),
                activate: Box::new(|item: &mut Self| item.send(Action::Show)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: "Quit BlueBubbles".into(),
                activate: Box::new(|item: &mut Self| item.send(Action::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Relaxed);
        // Never strand the user in an invisible window if their panel disappears.
        self.send(Action::Show);
        true
    }
}

impl Tray {
    pub fn start(ctx: &egui::Context) -> Self {
        let online = Arc::new(AtomicBool::new(false));
        let quit = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let mut item = Item {
            online: online.clone(),
            quit: quit.clone(),
            tx,
            ctx: ctx.clone(),
            icon: icon(),
            icon_file: String::new(),
        };
        std::thread::spawn(move || {
            // A real file works with panels that do not render pixmap-only items.
            // Cache the embedded asset so portable builds need no icon installation.
            item.icon_file = cache_icon().unwrap_or_default();
            loop {
                match item.clone().spawn() {
                    Ok(handle) => {
                        item.online.store(true, Ordering::Relaxed);
                        while !handle.is_closed() {
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                        item.online.store(false, Ordering::Relaxed);
                    }
                    Err(_) => {
                        item.online.store(false, Ordering::Relaxed);
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });
        Self { online, quit, rx }
    }
    pub fn online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }
    pub fn quitting(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }
    pub fn actions(&self) -> impl Iterator<Item = Action> + '_ {
        self.rx.try_iter()
    }
}

fn icon() -> Vec<u8> {
    let image = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("bundled icon");
    let mut data = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let offset =
                (((y * image.height / 32) * image.width + x * image.width / 32) * 4) as usize;
            let pixel = &image.rgba[offset..offset + 4];
            data.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
        }
    }
    data
}

fn cache_icon() -> Option<String> {
    let directories = directories::ProjectDirs::from("app", "bluebubbles", "rust-linux")?;
    let directory = directories.cache_dir().join("icons");
    std::fs::create_dir_all(&directory).ok()?;
    let path = directory.join("app.bluebubbles.RustLinux.png");
    let bytes = include_bytes!("../assets/icon.png");
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        let temporary = directory.join(format!("{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, bytes).ok()?;
        if std::fs::rename(&temporary, &path).is_err() {
            let _ = std::fs::remove_file(temporary);
            return None;
        }
    }
    Some(path.to_string_lossy().into_owned())
}
