use eframe::egui::{self, Color32, Rect, TextureHandle, Vec2};
use std::time::Duration;

pub struct Pet {
    awake: TextureHandle,
    asleep: TextureHandle,
    reduced: bool,
}

impl Pet {
    pub fn load(ctx: &egui::Context) -> Self {
        Self {
            awake: texture(ctx, "awake", include_bytes!("../assets/cat-awake.png")),
            asleep: texture(ctx, "asleep", include_bytes!("../assets/cat-asleep.png")),
            reduced: std::env::var_os("VOID_REDUCED_MOTION").is_some(),
        }
    }

    pub fn draw(&self, ui: &egui::Ui, bounds: Rect, awake: bool) {
        let time = ui.input(|input| input.time);
        let blink = !self.reduced && awake && time.rem_euclid(5.0) > 4.86;
        let texture = if awake && !blink {
            &self.awake
        } else {
            &self.asleep
        };
        let size = texture.size_vec2();
        let mut size = size * (78.0 / size.x).min(70.0 / size.y);
        if !self.reduced {
            // Anchor the breathing movement at the paws.
            size.y *= 1.0 + (time as f32 * 1.8).sin() * 0.018;
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let feet = bounds.center_bottom() - Vec2::new(0.0, 5.0);
        let rect = Rect::from_min_size(feet - Vec2::new(size.x * 0.5, size.y), size);
        ui.painter().image(
            texture.id(),
            rect,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("embedded Void sprite is a PNG")
        .into_rgba8();
    let (mut left, mut top, mut right, mut bottom) = (image.width(), image.height(), 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] >= 48 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    assert!(
        left < right && top < bottom,
        "embedded sprite has visible pixels"
    );
    let cropped =
        image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image();
    let sprite = image::DynamicImage::ImageRgba8(cropped)
        .resize(256, 256, image::imageops::FilterType::Triangle)
        .into_rgba8();
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [sprite.width() as usize, sprite.height() as usize],
            sprite.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}
