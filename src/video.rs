//! libmpv software rendering stays on one worker thread; only RGBA frames cross
//! into egui. Loading libmpv dynamically keeps the client usable without it.
use bluebubbles_linux::api::{Api, ApiResult};
use eframe::egui;
use std::{
    ffi::{c_char, c_int, c_void, CString},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

pub enum Command {
    Pause(bool),
    Seek(f64),
    Mute(bool),
}
#[derive(Default)]
pub struct Snapshot {
    pub frame: Option<egui::ColorImage>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub loaded: bool,
    pub error: Option<String>,
}
pub struct Player {
    pub snapshot: Arc<Mutex<Snapshot>>,
    tx: mpsc::Sender<Command>,
    cancelled: Arc<AtomicBool>,
}
impl Player {
    pub fn start(api: Api, guid: String, ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let state = snapshot.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        std::thread::spawn(move || {
            let result = (|| -> ApiResult<()> {
                let mut mpv = Engine::new()?;
                // Anonymous temporary storage has no pathname to leak after a crash.
                let mut file =
                    tempfile::tempfile().map_err(|_| "Cannot create temporary video buffer.")?;
                api.video_file(&guid, &mut file, &cancel)?;
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                use std::os::fd::AsRawFd;
                mpv.command(&["loadfile", &format!("/proc/self/fd/{}", file.as_raw_fd())])?;
                mpv.command(&["set", "pause", "no"])?;
                while !cancel.load(Ordering::Relaxed) {
                    for command in rx.try_iter() {
                        match command {
                            Command::Pause(paused) => {
                                mpv.command(&["set", "pause", if paused { "yes" } else { "no" }])?
                            }
                            Command::Seek(position) => {
                                mpv.command(&["seek", &position.to_string(), "absolute+exact"])?
                            }
                            Command::Mute(muted) => {
                                mpv.command(&["set", "mute", if muted { "yes" } else { "no" }])?
                            }
                        }
                    }
                    mpv.drain_events()?;
                    let frame = mpv.frame()?;
                    let position = mpv.number(c"time-pos").unwrap_or(0.0);
                    let duration = mpv.number(c"duration").unwrap_or(0.0);
                    let paused = mpv.flag(c"pause") || mpv.flag(c"eof-reached");
                    if let Ok(mut output) = state.lock() {
                        if frame.is_some() {
                            output.frame = frame;
                            output.loaded = true;
                        }
                        output.position = position;
                        output.duration = duration;
                        output.paused = paused;
                    }
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_millis(33));
                }
                Ok(())
            })();
            if let Err(error) = result {
                if !cancel.load(Ordering::Relaxed) {
                    if let Ok(mut state) = state.lock() {
                        state.error = Some(error);
                    }
                    ctx.request_repaint();
                }
            }
        });
        Self {
            snapshot,
            tx,
            cancelled,
        }
    }
    pub fn command(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}
struct Engine {
    // Keep the library loaded until render context and client handle are freed.
    _library: libloading::Library,
    handle: *mut c_void,
    renderer: *mut c_void,
    destroy: unsafe extern "C" fn(*mut c_void),
    free_render: unsafe extern "C" fn(*mut c_void),
    command: unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int,
    property: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    update: unsafe extern "C" fn(*mut c_void) -> u64,
    render: unsafe extern "C" fn(*mut c_void, *mut Param) -> c_int,
    wait: unsafe extern "C" fn(*mut c_void, f64) -> *const Event,
}
impl Engine {
    fn new() -> ApiResult<Self> {
        // SAFETY: signatures match the installed mpv client.h/render.h C API.
        // Every handle and render call is confined to this worker thread.
        unsafe {
            let library = libloading::Library::new("libmpv.so.2").map_err(|_| {
                "Inline video requires libmpv2. Install your distribution's libmpv2 package."
            })?;
            macro_rules! symbol {
                ($name:literal,$ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|_| "Installed libmpv is missing a required playback function.")?
                };
            }
            let create = symbol!("mpv_create", unsafe extern "C" fn() -> *mut c_void);
            let initialize = symbol!("mpv_initialize", unsafe extern "C" fn(*mut c_void) -> c_int);
            let option = symbol!(
                "mpv_set_option_string",
                unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int
            );
            let create_render = symbol!(
                "mpv_render_context_create",
                unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut Param) -> c_int
            );
            let callback = symbol!(
                "mpv_render_context_set_update_callback",
                unsafe extern "C" fn(
                    *mut c_void,
                    Option<unsafe extern "C" fn(*mut c_void)>,
                    *mut c_void,
                )
            );
            let mut result = Self {
                handle: std::ptr::null_mut(),
                renderer: std::ptr::null_mut(),
                destroy: symbol!("mpv_terminate_destroy", unsafe extern "C" fn(*mut c_void)),
                free_render: symbol!("mpv_render_context_free", unsafe extern "C" fn(*mut c_void)),
                command: symbol!(
                    "mpv_command",
                    unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int
                ),
                property: symbol!(
                    "mpv_get_property",
                    unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int
                ),
                update: symbol!(
                    "mpv_render_context_update",
                    unsafe extern "C" fn(*mut c_void) -> u64
                ),
                render: symbol!(
                    "mpv_render_context_render",
                    unsafe extern "C" fn(*mut c_void, *mut Param) -> c_int
                ),
                wait: symbol!(
                    "mpv_wait_event",
                    unsafe extern "C" fn(*mut c_void, f64) -> *const Event
                ),
                _library: library,
            };
            result.handle = create();
            if result.handle.is_null() {
                return Err("Cannot create video player.".into());
            }
            for (key, value) in [
                (c"vo", c"libmpv"),
                (c"config", c"no"),
                (c"load-scripts", c"no"),
                (c"ytdl", c"no"),
                (c"terminal", c"no"),
                (c"keep-open", c"yes"),
                (c"idle", c"yes"),
                (c"hwdec", c"no"),
                // Software libmpv rendering does not apply rotation metadata.
                // Apply it to the output pixels ourselves, exactly once.
                (c"video-rotate", c"no"),
                (c"pause", c"yes"),
            ] {
                if option(result.handle, key.as_ptr(), value.as_ptr()) < 0 {
                    return Err("Cannot configure inline video player.".into());
                }
            }
            if initialize(result.handle) < 0 {
                return Err("Cannot initialize inline video player.".into());
            }
            let mut params = [
                Param {
                    kind: 1,
                    data: c"sw".as_ptr() as *mut c_void,
                },
                Param {
                    kind: 0,
                    data: std::ptr::null_mut(),
                },
            ];
            if create_render(&mut result.renderer, result.handle, params.as_mut_ptr()) < 0 {
                return Err(
                    "Your libmpv version does not support inline software video rendering.".into(),
                );
            }
            unsafe extern "C" fn wake(_: *mut c_void) {}
            callback(result.renderer, Some(wake), std::ptr::null_mut());
            Ok(result)
        }
    }
    fn command(&mut self, args: &[&str]) -> ApiResult<()> {
        let strings = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Invalid video command.")?;
        let pointers = strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect::<Vec<_>>();
        // SAFETY: terminated argument vector and backing CStrings live through the call.
        if unsafe { (self.command)(self.handle, pointers.as_ptr()) } < 0 {
            Err("Video playback command failed.".into())
        } else {
            Ok(())
        }
    }
    fn number(&self, name: &std::ffi::CStr) -> Option<f64> {
        let mut value = 0.0_f64;
        // SAFETY: MPV_FORMAT_DOUBLE is 5 and value is a live aligned f64.
        (unsafe {
            (self.property)(
                self.handle,
                name.as_ptr(),
                5,
                &mut value as *mut _ as *mut c_void,
            )
        } >= 0)
            .then_some(value)
    }
    fn flag(&self, name: &std::ffi::CStr) -> bool {
        let mut value = 0_i32;
        // SAFETY: MPV_FORMAT_FLAG is 3 and value is a live C int.
        (unsafe {
            (self.property)(
                self.handle,
                name.as_ptr(),
                3,
                &mut value as *mut _ as *mut c_void,
            )
        }) >= 0
            && value != 0
    }
    fn drain_events(&self) -> ApiResult<()> {
        // SAFETY: event pointers remain valid until the next wait_event call.
        unsafe {
            loop {
                let event = &*(self.wait)(self.handle, 0.0);
                if event.id == 0 {
                    break;
                }
                if event.id == 7 && !event.data.is_null() {
                    let end = &*(event.data as *const EndFile);
                    if end.reason == 4 || end.error < 0 {
                        return Err(
                            "Video format could not be played. You can still save the attachment."
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(())
    }
    fn frame(&mut self) -> ApiResult<Option<egui::ColorImage>> {
        // SAFETY: context is live and only used on this thread.
        if unsafe { (self.update)(self.renderer) } & 1 == 0 {
            return Ok(None);
        }
        let width = self
            .number(c"video-out-params/dw")
            .unwrap_or(640.0)
            .max(1.0);
        let height = self
            .number(c"video-out-params/dh")
            .unwrap_or(360.0)
            .max(1.0);
        let scale = 640.0 / width.max(height);
        let w = (width * scale).round().clamp(1.0, 640.0) as usize;
        let h = (height * scale).round().clamp(1.0, 640.0) as usize;
        let rotation = self
            .number(c"video-dec-params/rotate")
            .unwrap_or(0.0)
            .round() as i32;
        let mut size = [w as i32, h as i32];
        let mut stride = w * 4;
        // u32 backing guarantees the four-byte alignment required by rgb0.
        let mut pixels = vec![0_u32; w * h];
        let mut params = [
            Param {
                kind: 17,
                data: size.as_mut_ptr().cast(),
            },
            Param {
                kind: 18,
                data: c"rgb0".as_ptr() as *mut c_void,
            },
            Param {
                kind: 19,
                data: (&mut stride as *mut usize).cast(),
            },
            Param {
                kind: 20,
                data: pixels.as_mut_ptr().cast(),
            },
            Param {
                kind: 0,
                data: std::ptr::null_mut(),
            },
        ];
        // SAFETY: buffers cover stride*height bytes and live for the entire call.
        if unsafe { (self.render)(self.renderer, params.as_mut_ptr()) } < 0 {
            return Err("Could not render video frame.".into());
        }
        let bytes = pixels
            .into_iter()
            .flat_map(|p| {
                let p = p.to_ne_bytes();
                [p[0], p[1], p[2], 255]
            })
            .collect::<Vec<_>>();
        let image = image::RgbaImage::from_raw(w as u32, h as u32, bytes)
            .ok_or("Invalid video frame dimensions.")?;
        let image = match rotation.rem_euclid(360) {
            90 => image::imageops::rotate90(&image),
            180 => image::imageops::rotate180(&image),
            270 => image::imageops::rotate270(&image),
            _ => image,
        };
        Ok(Some(egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        )))
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: destroy the render context before its owning mpv handle.
        unsafe {
            if !self.renderer.is_null() {
                (self.free_render)(self.renderer);
            }
            if !self.handle.is_null() {
                (self.destroy)(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Run tests/video_orientation_smoke.py with libmpv2 and FFmpeg"]
    fn video_rotation_metadata_is_respected() {
        let directory = std::env::var("BB_ROTATION_DIR").unwrap();
        for rotation in [0, 90, 180, 270] {
            let mut engine = Engine::new().unwrap();
            engine
                .command(&["loadfile", &format!("{directory}/{rotation}.mp4")])
                .unwrap();
            engine.command(&["set", "pause", "no"]).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                engine.drain_events().unwrap();
                if let Some(frame) = engine.frame().unwrap() {
                    if engine.number(c"time-pos").unwrap_or_default() > 0.1 {
                        let [w, h] = frame.size;
                        assert_eq!(
                            [w, h],
                            if rotation % 180 == 0 {
                                [640, 360]
                            } else {
                                [360, 640]
                            }
                        );
                        let red_quadrants = match rotation {
                            0 => [true, true, false, false],
                            90 => [false, true, false, true],
                            180 => [false, false, true, true],
                            _ => [true, false, true, false],
                        };
                        for ((x, y), red) in [
                            (w / 4, h / 4),
                            (3 * w / 4, h / 4),
                            (w / 4, 3 * h / 4),
                            (3 * w / 4, 3 * h / 4),
                        ]
                        .into_iter()
                        .zip(red_quadrants)
                        {
                            let color = frame.pixels[y * w + x];
                            assert!(
                                if red {
                                    color.r() > 180 && color.b() < 100
                                } else {
                                    color.b() > 180 && color.r() < 100
                                },
                                "Wrong orientation at {rotation} degrees: {color:?}"
                            );
                        }
                        break;
                    }
                }
                assert!(std::time::Instant::now() < deadline, "No orientation frame");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    #[test]
    #[ignore = "Requires libmpv2 and BB_TEST_VIDEO pointing at an eight-second test clip"]
    fn inline_video_renders_pauses_and_seeks() {
        let mut engine = Engine::new().unwrap();
        engine
            .command(&["loadfile", &std::env::var("BB_TEST_VIDEO").unwrap()])
            .unwrap();
        engine.command(&["set", "pause", "no"]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut colorful = false;
        while std::time::Instant::now() < deadline {
            engine.drain_events().unwrap();
            if let Some(frame) = engine.frame().unwrap() {
                colorful |= frame.pixels.iter().any(|p| p.r() > 180 && p.g() < 100);
            }
            if colorful && engine.number(c"time-pos").unwrap_or_default() > 0.3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(colorful, "No decoded color frames");
        assert!(engine.number(c"duration").unwrap() > 7.0);
        engine.command(&["set", "pause", "yes"]).unwrap();
        assert!(engine.flag(c"pause"));
        engine.command(&["seek", "4", "absolute+exact"]).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            engine.drain_events().unwrap();
            let _ = engine.frame().unwrap();
            if engine.number(c"time-pos").unwrap_or_default() >= 3.9 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "Seek timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
