use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{FftFixedIn, Resampler};

use crate::SAMPLE_RATE;

/// Rolling audio kept from before the hotkey press — the user starts
/// speaking as they press, and stream startup would otherwise clip it.
const PREROLL_MS: u64 = 300;

struct Inner {
    preroll: VecDeque<f32>,
    active: Option<Vec<f32>>,
}

/// Microphone capture with a small pre-roll ring. `begin()`/`end()` bracket
/// an utterance. The daemon opens this on demand and drops it after a short
/// idle window so the OS mic-in-use indicator only shows during dictation.
/// Capture is at the device's native rate, downmixed to mono; resampling
/// to 16 kHz happens per `snapshot()`/`end()` call.
pub struct Capture {
    _stream: cpal::Stream,
    inner: Arc<Mutex<Inner>>,
    device_rate: u32,
}

impl Capture {
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device")?;
        let config = device
            .default_input_config()
            .context("no default input config")?;
        let device_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        log::info!(
            "mic open: '{}' at {} Hz, {} ch (warm stream, {}ms pre-roll)",
            device.name().unwrap_or_default(),
            device_rate,
            channels,
            PREROLL_MS
        );

        let preroll_cap = (device_rate as u64 * PREROLL_MS / 1000) as usize;
        let inner = Arc::new(Mutex::new(Inner {
            preroll: VecDeque::with_capacity(preroll_cap),
            active: None,
        }));
        let cb_inner = inner.clone();
        let err_fn = |e| log::error!("audio stream error: {e}");

        let stream = device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _: &_| {
                    let mut inner = cb_inner.lock().unwrap();
                    let mono = data
                        .chunks_exact(channels)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32);
                    if let Some(active) = inner.active.as_mut() {
                        active.extend(mono);
                    } else {
                        inner.preroll.extend(mono);
                        while inner.preroll.len() > preroll_cap {
                            inner.preroll.pop_front();
                        }
                    }
                },
                err_fn,
                None,
            )
            .context("building input stream")?;
        stream.play().context("starting input stream")?;

        Ok(Self {
            _stream: stream,
            inner,
            device_rate,
        })
    }

    /// Arms recording; the pre-roll becomes the start of the utterance.
    pub fn begin(&self) {
        let mut inner = self.inner.lock().unwrap();
        let mut buf: Vec<f32> = inner.preroll.drain(..).collect();
        buf.reserve(self.device_rate as usize * 10);
        inner.active = Some(buf);
    }

    /// Copy of the utterance so far (16 kHz mono) without disarming —
    /// used for rolling transcription passes while the key is held.
    pub fn snapshot(&self) -> Result<Vec<f32>> {
        self.snapshot_from(0)
    }

    /// Like `snapshot`, but starting `from` samples into the utterance.
    ///
    /// Streaming transcribes a bounded window rather than the whole utterance:
    /// re-transcribing everything makes each pass cost grow without limit, so a
    /// long utterance eventually costs more per pass than the interval between
    /// passes, and the model has a hard duration ceiling besides. `from` is an
    /// index into the *captured* samples, i.e. at the device rate.
    pub fn snapshot_from(&self, from: usize) -> Result<Vec<f32>> {
        let inner = self.inner.lock().unwrap();
        let active = inner.active.as_ref().context("snapshot() without begin()")?;
        let start = from.min(active.len());
        let samples = active[start..].to_vec();
        drop(inner);
        if self.device_rate == SAMPLE_RATE {
            return Ok(samples);
        }
        resample(&samples, self.device_rate, SAMPLE_RATE)
    }

    /// Samples captured so far, at the device rate — the unit `snapshot_from`
    /// expects.
    pub fn armed_samples(&self) -> usize {
        self.inner
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|a| a.len())
            .unwrap_or(0)
    }

    /// Capture rate, so callers can convert seconds to sample offsets.
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    /// Disarms and returns the utterance as 16 kHz mono samples.
    pub fn end(&self) -> Result<Vec<f32>> {
        let samples = self
            .inner
            .lock()
            .unwrap()
            .active
            .take()
            .context("end() without begin()")?;
        if self.device_rate == SAMPLE_RATE {
            return Ok(samples);
        }
        resample(&samples, self.device_rate, SAMPLE_RATE)
    }

    pub fn cancel(&self) {
        self.inner.lock().unwrap().active = None;
    }

    pub fn armed_secs(&self) -> f32 {
        self.inner
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|a| a.len() as f32 / self.device_rate as f32)
            .unwrap_or(0.0)
    }
}

fn resample(input: &[f32], from: u32, to: u32) -> Result<Vec<f32>> {
    const CHUNK: usize = 1024;
    let mut rs = FftFixedIn::<f32>::new(from as usize, to as usize, CHUNK, 2, 1)
        .context("creating resampler")?;
    let mut out = Vec::with_capacity(input.len() * to as usize / from as usize + CHUNK);
    for chunk in input.chunks(CHUNK) {
        let padded;
        let chunk = if chunk.len() == CHUNK {
            chunk
        } else {
            // rubato's fixed-input resampler needs full chunks; zero-pad the tail
            padded = {
                let mut p = chunk.to_vec();
                p.resize(CHUNK, 0.0);
                p
            };
            &padded
        };
        let result = rs.process(&[chunk], None).context("resampling")?;
        out.extend_from_slice(&result[0]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live path captures at the device rate (48 kHz on a MacBook) and
    /// resamples every snapshot and the final utterance down to 16 kHz. File
    /// based testing feeds 16 kHz straight in and never exercises this, so it
    /// gets its own coverage — a resampler that silently drops or zeroes long
    /// inputs would empty out exactly the final pass and nothing else.
    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn tone(secs: f32, rate: u32) -> Vec<f32> {
        let n = (secs * rate as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                0.3 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn resamples_48k_to_16k_preserving_length_and_level() {
        for secs in [1.0f32, 30.0, 75.0, 120.0] {
            let input = tone(secs, 48_000);
            let out = resample(&input, 48_000, SAMPLE_RATE).expect("resample");
            let expected = (secs * SAMPLE_RATE as f32) as usize;
            let drift = (out.len() as f32 - expected as f32).abs() / expected as f32;
            assert!(
                drift < 0.02,
                "{secs}s: got {} samples, expected ~{expected} ({:.1}% off)",
                out.len(),
                drift * 100.0
            );
            let level = rms(&out);
            assert!(
                level > 0.1,
                "{secs}s: output is near-silent (rms {level:.4}) — input rms {:.4}",
                rms(&input)
            );
        }
    }

    #[test]
    fn resampled_output_is_not_mostly_silence_at_the_tail() {
        // a long input whose *end* goes quiet must still carry signal earlier on
        let input = tone(75.0, 48_000);
        let out = resample(&input, 48_000, SAMPLE_RATE).expect("resample");
        let quarter = out.len() / 4;
        for (i, part) in out.chunks(quarter.max(1)).take(4).enumerate() {
            assert!(rms(part) > 0.1, "quarter {i} is silent (rms {:.4})", rms(part));
        }
    }
}
