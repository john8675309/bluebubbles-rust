use eframe::egui::{self, Vec2};

#[derive(Default)]
pub struct DisplayScaling {
    monitor: Option<(u32, u32, u32)>,
    resize: Option<(f32, Vec2)>,
}

impl DisplayScaling {
    pub fn update(&mut self, ctx: &egui::Context) {
        let viewport = ctx.input(|i| i.viewport().clone());
        let Some(size) = viewport.monitor_size else {
            return;
        };
        let Some(native) = viewport.native_pixels_per_point else {
            return;
        };
        // egui reports monitor dimensions in UI points, including our own zoom.
        // Convert back to physical pixels so changing zoom cannot flip detection.
        let physical = size * ctx.pixels_per_point();
        if !physical.is_finite()
            || physical.min_elem() <= 0.0
            || !native.is_finite()
            || native <= 0.0
        {
            return;
        }
        let monitor = (
            physical.x.round() as u32,
            physical.y.round() as u32,
            (native * 1000.0).round() as u32,
        );
        if self.monitor != Some(monitor) {
            self.monitor = Some(monitor);
            let zoom = automatic_zoom(physical, native);
            if (ctx.zoom_factor() - zoom).abs() > 0.01 {
                self.resize = viewport.inner_rect.map(|rect| (zoom, rect.size()));
                ctx.set_zoom_factor(zoom);
                ctx.request_repaint();
                return;
            }
        }
        // Zoom takes effect on the next pass. Resize afterward to retain the
        // window's usable logical size instead of squeezing a 2x UI into 1100px.
        if let Some((zoom, desired)) = self.resize {
            if (ctx.zoom_factor() - zoom).abs() < 0.01 {
                self.resize = None;
                if viewport.maximized != Some(true) && viewport.fullscreen != Some(true) {
                    let available = physical / (native * zoom) * 0.9;
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired.min(available)));
                }
            }
        }
    }
}

fn automatic_zoom(physical: Vec2, native: f32) -> f32 {
    // Honor explicit desktop HiDPI/fractional scaling rather than doubling it.
    // Use both dimensions: a 3840x1080 ultrawide is not a 4K HiDPI display.
    if native <= 1.05 && physical.min_elem() >= 2160.0 && physical.max_elem() >= 3840.0 {
        2.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unscaled_4k_and_portrait_get_double_size() {
        for size in [
            egui::vec2(3840.0, 2160.0),
            egui::vec2(2160.0, 3840.0),
            egui::vec2(4096.0, 2160.0),
        ] {
            assert_eq!(automatic_zoom(size, 1.0), 2.0);
        }
    }

    #[test]
    fn desktop_scaling_is_not_multiplied() {
        for native in [1.25, 1.5, 2.0] {
            assert_eq!(automatic_zoom(egui::vec2(3840.0, 2160.0), native), 1.0);
        }
    }

    #[test]
    fn normal_displays_and_ultrawides_keep_normal_size() {
        for size in [
            egui::vec2(1920.0, 1080.0),
            egui::vec2(2560.0, 1440.0),
            egui::vec2(3840.0, 1080.0),
        ] {
            assert_eq!(automatic_zoom(size, 1.0), 1.0);
        }
    }

    #[test]
    fn monitor_changes_do_not_cause_zoom_oscillation() {
        let ctx = egui::Context::default();
        let mut scaling = DisplayScaling::default();
        for (physical, native, expected) in [
            (egui::vec2(3840.0, 2160.0), 1.0, 2.0),
            (egui::vec2(1920.0, 1080.0), 1.0, 1.0),
            (egui::vec2(3840.0, 2160.0), 2.0, 1.0),
        ] {
            for _ in 0..5 {
                let mut input = egui::RawInput::default();
                let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
                viewport.native_pixels_per_point = Some(native);
                viewport.monitor_size = Some(physical / (native * ctx.zoom_factor()));
                let _ = ctx.run(input, |ctx| scaling.update(ctx));
            }
            assert_eq!(ctx.zoom_factor(), expected);
        }
    }
}
