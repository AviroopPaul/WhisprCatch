//! The input device list, read off the UI thread.
//!
//! Listing devices asks each one for its formats, which took 364ms on a laptop
//! with a Bluetooth bar and a Continuity iPhone in range. Done inline that would
//! freeze whichever window asked, so a window keeps the last answer and asks for
//! a fresh one in the background.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

#[derive(Clone, Default)]
pub struct MicList {
    names: Arc<Mutex<Vec<String>>>,
    reading: Arc<AtomicBool>,
}

impl MicList {
    /// Start a read unless one is running. Repaints `ctx` when it lands.
    pub fn refresh(&self, ctx: &egui::Context) {
        if self.reading.swap(true, Ordering::SeqCst) {
            return;
        }
        let list = self.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let names = wc_core::audio::input_device_names();
            *list.names.lock().unwrap() = names;
            list.reading.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    pub fn get(&self) -> Vec<String> {
        self.names.lock().unwrap().clone()
    }
}
