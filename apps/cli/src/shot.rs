//! Dev-only window capture: set `WC_SHOT=/path/out.png` (and optionally
//! `WC_SHOT_FRAMES=n`, default 30) to save a PNG of the window after n
//! frames and exit. Used to produce design screenshots in CI-less
//! environments; not user-facing.

use eframe::egui;

pub struct Shot {
    path: Option<String>,
    after: u64,
    frames: u64,
    requested: bool,
}

impl Shot {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var("WC_SHOT").ok(),
            after: std::env::var("WC_SHOT_FRAMES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            frames: 0,
            requested: false,
        }
    }

    /// Call once per frame from `App::update`. Saves + exits when done.
    pub fn tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.frames += 1;
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        if self.frames >= self.after && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let buf: image::RgbaImage =
                image::RgbaImage::from_raw(w as u32, h as u32, img.as_raw().to_vec())
                    .expect("screenshot buffer");
            buf.save(&path).expect("write screenshot");
            eprintln!("WC_SHOT saved {path} ({w}x{h} px, viewport {:?})", ctx.screen_rect().size());
            std::process::exit(0);
        }
    }
}
