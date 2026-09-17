use crate::video::{Command, Player};
use bluebubbles_linux::{
    api::{Api, ApiResult},
    model::Attachment,
};
use eframe::egui;
use std::{
    collections::HashMap,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

enum ImageState {
    Loading,
    Ready(ImageAsset),
    Failed(String),
}
struct ImageEntry {
    state: ImageState,
    touched: Instant,
}
struct ActiveVideo {
    guid: String,
    player: Player,
    texture: Option<egui::TextureHandle>,
    muted: bool,
    seek_position: Option<f64>,
}
pub struct Media {
    images: HashMap<String, ImageEntry>,
    tx: mpsc::Sender<(String, ApiResult<DecodedImage>)>,
    rx: mpsc::Receiver<(String, ApiResult<DecodedImage>)>,
    loading: usize,
    video: Option<ActiveVideo>,
    enlarged: Option<String>,
}
impl Default for Media {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            images: HashMap::new(),
            tx,
            rx,
            loading: 0,
            video: None,
            enlarged: None,
        }
    }
}
impl Media {
    pub fn stop_video(&mut self) {
        self.video = None;
    }
    pub fn update(&mut self, ctx: &egui::Context) {
        for (guid, result) in self.rx.try_iter() {
            self.loading = self.loading.saturating_sub(1);
            if let Some(entry) = self.images.get_mut(&guid) {
                entry.touched = Instant::now();
                entry.state = match result {
                    Ok(decoded) => {
                        let decoded_delay = decoded.delay;
                        ImageState::Ready(ImageAsset {
                            texture: ctx.load_texture(
                                format!("attachment-{guid}"),
                                decoded.first.clone(),
                                egui::TextureOptions::LINEAR,
                            ),
                            decoded,
                            next_frame: Instant::now() + decoded_delay,
                            pending: false,
                            error: None,
                        })
                    }
                    Err(error) => ImageState::Failed(error),
                };
            }
        }
        while self
            .images
            .values()
            .map(|entry| match &entry.state {
                ImageState::Ready(asset) => asset.decoded.bytes(),
                _ => 0,
            })
            .sum::<usize>()
            > 96 * 1024 * 1024
        {
            let oldest = self
                .images
                .iter()
                .filter(|(guid, entry)| {
                    matches!(entry.state, ImageState::Ready(_))
                        && entry.touched.elapsed() > Duration::from_secs(1)
                        && self.enlarged.as_ref() != Some(*guid)
                })
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(guid, _)| guid.clone());
            if self
                .images
                .values()
                .filter(|entry| matches!(entry.state, ImageState::Ready(_)))
                .count()
                <= 1
            {
                break;
            }
            if let Some(guid) = oldest {
                self.images.remove(&guid);
            } else {
                break;
            }
        }
        if let Some(guid) = &self.enlarged {
            let mut open = true;
            if let Some(ImageEntry {
                state: ImageState::Ready(asset),
                touched,
            }) = self.images.get_mut(guid)
            {
                *touched = Instant::now();
                asset.advance(ctx);
                egui::Window::new("Image")
                    .open(&mut open)
                    .resizable(true)
                    .show(ctx, |ui| {
                        ui.add(
                            egui::Image::new(&asset.texture)
                                .max_size(egui::vec2(900.0, 650.0))
                                .maintain_aspect_ratio(true),
                        );
                    });
            } else {
                open = false;
            }
            if !open {
                self.enlarged = None;
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, api: Option<&Api>, attachment: &Attachment) {
        let kind = kind(attachment);
        if kind == 0 {
            return;
        }
        if kind == 2 {
            self.video_ui(ui, api, attachment);
            return;
        }
        let visible = ui.is_rect_visible(egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(420.0, 300.0),
        ));
        if !self.images.contains_key(&attachment.guid) && visible && self.loading < 3 {
            if let Some(api) = api {
                if self.images.len() >= 24 {
                    if let Some(old) = self
                        .images
                        .iter()
                        .filter(|(_, e)| !matches!(e.state, ImageState::Loading))
                        .min_by_key(|(_, e)| e.touched)
                        .map(|(key, _)| key.clone())
                    {
                        self.images.remove(&old);
                    }
                }
                let guid = attachment.guid.clone();
                self.images.insert(
                    guid.clone(),
                    ImageEntry {
                        state: ImageState::Loading,
                        touched: Instant::now(),
                    },
                );
                self.loading += 1;
                let api = api.clone();
                let tx = self.tx.clone();
                let ctx = ui.ctx().clone();
                let gif = attachment
                    .mime_type
                    .as_deref()
                    .is_some_and(|mime| mime.eq_ignore_ascii_case("image/gif"))
                    || attachment
                        .transfer_name
                        .as_deref()
                        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".gif"));
                std::thread::spawn(move || {
                    let result = if gif {
                        api.animated_image(&guid)
                    } else {
                        api.image_preview(&guid)
                    }
                    .and_then(decode);
                    let _ = tx.send((guid, result));
                    ctx.request_repaint();
                });
            }
        }
        let mut retry = false;
        match self.images.get_mut(&attachment.guid) {
            Some(entry) => {
                if visible {
                    entry.touched = Instant::now();
                }
                match &mut entry.state {
                    ImageState::Ready(asset) => {
                        if visible {
                            asset.advance(ui.ctx());
                        }
                        if let Some(error) = &asset.error {
                            ui.label(error);
                        }
                        let texture = &asset.texture;
                        let response = ui.add(
                            egui::Image::new(texture)
                                .max_size(egui::vec2(ui.available_width().min(420.0), 300.0))
                                .maintain_aspect_ratio(true)
                                .sense(egui::Sense::click()),
                        );
                        if response.clicked() {
                            self.enlarged = Some(attachment.guid.clone());
                        }
                    }
                    ImageState::Loading => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading image…");
                        });
                        ui.add_space(120.0);
                    }
                    ImageState::Failed(error) => {
                        ui.label(error.as_str());
                        retry = ui.button("Retry image").clicked();
                    }
                }
            }
            None => {
                ui.label("Image preview");
                ui.add_space(120.0);
            }
        }
        if retry {
            self.images.remove(&attachment.guid);
        }
    }
    fn video_ui(&mut self, ui: &mut egui::Ui, api: Option<&Api>, attachment: &Attachment) {
        if self
            .video
            .as_ref()
            .is_none_or(|video| video.guid != attachment.guid)
        {
            ui.allocate_ui(egui::vec2(ui.available_width().min(420.0), 110.0), |ui| {
                ui.vertical_centered(|ui| {
                    ui.label("Video");
                    if ui.button("▶ Play video").clicked() {
                        if let Some(api) = api {
                            self.video = Some(ActiveVideo {
                                guid: attachment.guid.clone(),
                                player: Player::start(
                                    api.clone(),
                                    attachment.guid.clone(),
                                    ui.ctx().clone(),
                                ),
                                texture: None,
                                muted: false,
                                seek_position: None,
                            });
                        }
                    }
                });
            });
            return;
        }
        let video = self.video.as_mut().unwrap();
        let mut stop = false;
        if let Ok(mut state) = video.player.snapshot.lock() {
            if let Some(frame) = state.frame.take() {
                if let Some(texture) = &mut video.texture {
                    texture.set(frame, egui::TextureOptions::LINEAR);
                } else {
                    video.texture = Some(ui.ctx().load_texture(
                        "inline-video",
                        frame,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            if let Some(error) = &state.error {
                ui.label(error.as_str());
                stop = ui.button("Close player").clicked();
            } else {
                if let Some(texture) = &video.texture {
                    ui.add(
                        egui::Image::new(texture)
                            .max_size(egui::vec2(ui.available_width().min(480.0), 320.0))
                            .maintain_aspect_ratio(true),
                    );
                }
                if !state.loaded {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading video…");
                    });
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(if state.paused { "▶ Play" } else { "Pause" })
                        .clicked()
                    {
                        if state.duration > 0.0 && state.position >= state.duration - 0.1 {
                            video.player.command(Command::Seek(0.0));
                        }
                        video.player.command(Command::Pause(!state.paused));
                    }
                    if ui.checkbox(&mut video.muted, "Mute").changed() {
                        video.player.command(Command::Mute(video.muted));
                    }
                    stop |= ui.button("Stop").clicked();
                });
                if state.duration.is_finite() && state.duration > 0.0 {
                    let mut position = video
                        .seek_position
                        .unwrap_or(state.position)
                        .clamp(0.0, state.duration);
                    let response =
                        ui.add(egui::Slider::new(&mut position, 0.0..=state.duration).suffix(" s"));
                    if response.dragged() {
                        video.seek_position = Some(position);
                    }
                    if response.changed() {
                        video.player.command(Command::Seek(position));
                    }
                    if !response.dragged() {
                        video.seek_position = None;
                    }
                }
            }
        }
        if stop {
            self.video = None;
        }
    }
}
fn kind(attachment: &Attachment) -> u8 {
    let mime = attachment.mime_type.as_deref().unwrap_or_default();
    if mime.starts_with("image/") {
        return 1;
    }
    if mime.starts_with("video/") {
        return 2;
    }
    let extension = attachment
        .transfer_name
        .as_deref()
        .and_then(|name| std::path::Path::new(name).extension())
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" => 1,
        "mp4" | "mov" | "m4v" | "webm" | "mkv" => 2,
        _ => 0,
    }
}
struct DecodedImage {
    first: Arc<egui::ColorImage>,
    delay: Duration,
    stream: Option<GifStream>,
    memory: usize,
}
impl DecodedImage {
    fn bytes(&self) -> usize {
        self.memory
    }
}
struct ImageAsset {
    decoded: DecodedImage,
    texture: egui::TextureHandle,
    next_frame: Instant,
    pending: bool,
    error: Option<String>,
}
impl ImageAsset {
    fn advance(&mut self, ctx: &egui::Context) {
        let Some(stream) = self.decoded.stream.as_ref() else {
            return;
        };
        if !self.pending {
            if stream.requests.try_send(()).is_err() {
                return;
            }
            self.pending = true;
        }
        let now = Instant::now();
        if now >= self.next_frame {
            match stream.frames.try_recv() {
                Ok(Ok(frame)) => {
                    self.texture.set(frame.pixels, egui::TextureOptions::LINEAR);
                    self.next_frame = now + frame.delay;
                    self.pending = false;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.decoded.stream = None;
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.decoded.stream = None;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.pending {
            if let Some(stream) = self.decoded.stream.as_ref() {
                self.pending = stream.requests.try_send(()).is_ok();
            }
        }
        // Prefetch at most one frame. Offscreen images make no further requests.
        ctx.request_repaint_after(
            self.next_frame
                .saturating_duration_since(now)
                .max(Duration::from_millis(10)),
        );
    }
}
struct GifFrame {
    pixels: Arc<egui::ColorImage>,
    delay: Duration,
}
struct GifStream {
    requests: mpsc::SyncSender<()>,
    frames: mpsc::Receiver<ApiResult<GifFrame>>,
}
struct GifReader {
    bytes: Arc<[u8]>,
    frames: image::Frames<'static>,
    memory: usize,
}
impl GifReader {
    fn new(bytes: Arc<[u8]>) -> ApiResult<Self> {
        use image::{AnimationDecoder, ImageDecoder};
        let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes.clone()))
            .map_err(|_| "Could not read GIF.")?;
        decoder
            .set_limits(image_limits())
            .map_err(|_| "GIF dimensions are too large for inline playback.")?;
        let (width, height) = decoder.dimensions();
        // Account for full-size disposal/compositing buffers, not just thumbnails.
        let memory = u64::from(width) * u64::from(height) * 16;
        if memory > 128 * 1024 * 1024 {
            return Err("GIF dimensions are too large for inline playback.".into());
        }
        let memory = memory as usize + bytes.len() + 3 * 480 * 480 * 4;
        Ok(Self {
            bytes,
            frames: decoder.into_frames(),
            memory,
        })
    }
    fn next(&mut self) -> ApiResult<GifFrame> {
        let frame = match self.frames.next() {
            Some(frame) => frame,
            None => {
                *self = Self::new(self.bytes.clone())?;
                self.frames.next().ok_or("GIF contains no frames.")?
            }
        }
        .map_err(|_| "Could not decode GIF frames. Use Save to download it.")?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = Duration::from_secs_f64(
            (f64::from(numerator) / f64::from(denominator) / 1000.0).max(0.02),
        );
        let buffer = frame.into_buffer();
        let width = buffer.width().min(480);
        let height = buffer.height().min(480);
        let pixels = image::DynamicImage::ImageRgba8(buffer)
            .thumbnail(width, height)
            .into_rgba8();
        Ok(GifFrame {
            pixels: color_image(pixels),
            delay,
        })
    }
}
fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}
fn decode(bytes: Vec<u8>) -> ApiResult<DecodedImage> {
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        let (requests, work) = mpsc::sync_channel(1);
        let (output, frames) = mpsc::sync_channel(1);
        let (ready, initial) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let initialized = (|| -> ApiResult<_> {
                let mut reader = GifReader::new(bytes.into())?;
                let first = reader.next()?;
                Ok((reader, first))
            })();
            let mut reader = match initialized {
                Ok((reader, first)) => {
                    if ready.send(Ok((first, reader.memory))).is_err() {
                        return;
                    }
                    reader
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            while work.recv().is_ok() {
                let frame = reader.next();
                let failed = frame.is_err();
                if output.send(frame).is_err() || failed {
                    break;
                }
            }
        });
        let (first, memory) = initial.recv().map_err(|_| "GIF decoder stopped.")??;
        Ok(DecodedImage {
            first: first.pixels,
            delay: first.delay,
            stream: Some(GifStream { requests, frames }),
            memory,
        })
    } else {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "Could not read image format.")?;
        reader.limits(image_limits());
        let pixels = reader
            .decode()
            .map_err(|_| "Cannot display this image inline. Use Save to download it.")?
            .thumbnail(1200, 1200)
            .into_rgba8();
        let memory = pixels.as_raw().len();
        Ok(DecodedImage {
            first: color_image(pixels),
            delay: Duration::ZERO,
            stream: None,
            memory,
        })
    }
}
fn color_image(pixels: image::RgbaImage) -> Arc<egui::ColorImage> {
    Arc::new(egui::ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        pixels.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gif(count: usize, width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for index in 0..count {
                let color = if index % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(width, height, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(
                            if index % 2 == 0 { 100 } else { 300 },
                            1,
                        ),
                    ))
                    .unwrap();
            }
        }
        bytes
    }
    #[test]
    fn long_gif_streams_all_frames_and_loops_with_constant_memory() {
        let mut reader = GifReader::new(gif(302, 4, 3).into()).unwrap();
        let memory = reader.memory;
        for index in 0..306 {
            let frame = reader.next().unwrap();
            assert_eq!(
                frame.pixels.pixels[0],
                if index % 2 == 0 {
                    egui::Color32::RED
                } else {
                    egui::Color32::BLUE
                }
            );
            assert_eq!(
                frame.delay,
                Duration::from_millis(if index % 2 == 0 { 100 } else { 300 })
            );
            assert_eq!(reader.memory, memory);
        }
    }
    #[test]
    fn gif_over_old_decoded_memory_limit_plays_on_demand() {
        // 40 full 480x480 RGBA frames exceed the former 32 MiB ceiling.
        let decoded = decode(gif(40, 480, 480)).unwrap();
        assert!(decoded.memory < 8 * 1024 * 1024);
        let stream = decoded.stream.unwrap();
        for index in 1..43 {
            stream.requests.send(()).unwrap();
            let frame = stream
                .frames
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(
                frame.pixels.pixels[0],
                if index % 2 == 0 {
                    egui::Color32::RED
                } else {
                    egui::Color32::BLUE
                }
            );
        }
        // Drop disconnects the worker, including when idle waiting for a request.
    }
    #[test]
    fn malformed_gif_is_rejected() {
        assert!(decode(b"GIF89a truncated".to_vec()).is_err());
    }
}
