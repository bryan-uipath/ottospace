//! A test run's recording of its own window (`record_start`, `record_stop`), for
//! `scripts/shot --record`. ScreenCaptureKit streams the window, all of it though only a sliver
//! is on screen, at up to 60 frames a second; each frame it sends (one per change) is written
//! as raw BGRA, `<n>.bgra`, with a line in `frames.jsonl`: `{"n":0,"ms":..,"w":1400,"h":900}`.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use objc2_core_foundation::CGSize;
use screencapturekit::prelude::*;

pub struct Recording {
    stream: SCStream,
    frames: Arc<AtomicU64>,
}

struct Writer {
    dir: PathBuf,
    index: Mutex<File>,
    frames: Arc<AtomicU64>,
    size: (usize, usize),
}

impl SCStreamOutputTrait for Writer {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, _: SCStreamOutputType) {
        // Idle samples ("nothing changed") carry no image.
        let Some(buffer) = sample.pixel_buffer() else { return };
        let Ok(guard) = buffer.lock_read_only() else { return };
        let (base, stride) = (guard.base_address(), guard.bytes_per_row());
        let (w, h) = self.size;
        if base.is_null() || guard.width() != w || guard.height() != h || stride < w * 4 {
            return;
        }
        // Rows without their padding: `bytes_per_row` counts it.
        let all = unsafe { std::slice::from_raw_parts(base, stride * h) };
        let pixels: Vec<u8> = all.chunks_exact(stride).flat_map(|row| &row[..w * 4]).copied().collect();
        let n = self.frames.fetch_add(1, Relaxed);
        if fs::write(self.dir.join(format!("{n}.bgra")), pixels).is_err() {
            return;
        }
        let line = serde_json::json!({ "n": n, "ms": crate::now_ms(), "w": w, "h": h });
        // One write a line: a reader never sees half of one.
        if let Ok(mut index) = self.index.lock() {
            let _ = index.write_all(format!("{line}\n").as_bytes());
        }
    }
}

/// Starts recording window `id`, `size` points, at one pixel a point, into the folder `dir`.
pub fn start(id: u32, size: CGSize, dir: PathBuf) -> Result<Recording, String> {
    let window = ottospace_mac::capture::sc_window(id)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let index = File::create(dir.join("frames.jsonl")).map_err(|e| e.to_string())?;
    let (w, h) = (size.width.round().max(1.0) as usize, size.height.round().max(1.0) as usize);
    let filter = SCContentFilter::create().with_window(&window).build().map_err(|e| format!("{e:?}"))?;
    let config = SCStreamConfiguration::new()
        .with_width(w as u32)
        .with_height(h as u32)
        .with_scales_to_fit(true)
        .with_pixel_format(PixelFormat::BGRA)
        .with_fps(60);
    let frames = Arc::new(AtomicU64::new(0));
    let writer = Writer { dir, index: Mutex::new(index), frames: frames.clone(), size: (w, h) };
    let mut stream = SCStream::new(&filter, &config).map_err(|e| format!("{e:?}"))?;
    stream.add_output_handler(writer, SCStreamOutputType::Screen).map_err(|e| format!("{e:?}"))?;
    stream.start_capture().map_err(|e| format!("{e:?}"))?;
    Ok(Recording { stream, frames })
}

impl Recording {
    /// Stops it; returns how many frames were written.
    pub fn stop(self) -> u64 {
        let _ = self.stream.stop_capture();
        self.frames.load(Relaxed)
    }
}
