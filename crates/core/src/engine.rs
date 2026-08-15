use std::path::Path;

use anyhow::{Context, Result};
use transcribe_rs::onnx::moonshine::{MoonshineModel, MoonshineParams, MoonshineVariant};
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

pub use wc_models::ModelId;

use crate::SAMPLE_RATE;

/// A loaded STT model. Load once, transcribe many times. The variant is chosen
/// by the user (settings → Model); both run int8 ONNX via the same ORT stack.
pub enum Engine {
    Parakeet(ParakeetModel),
    Moonshine(MoonshineModel),
}

impl Engine {
    /// Loads `model` from `model_dir` (the directory containing its ONNX files).
    pub fn load(model: ModelId, model_dir: &Path) -> Result<Self> {
        let dir = model_dir.to_path_buf();
        match model {
            ModelId::Parakeet => {
                let m = ParakeetModel::load(&dir, &Quantization::Int8)
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .with_context(|| format!("loading Parakeet model from {}", dir.display()))?;
                Ok(Engine::Parakeet(m))
            }
            ModelId::Moonshine => {
                let m = MoonshineModel::load(&dir, MoonshineVariant::Base, &Quantization::Int8)
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .with_context(|| format!("loading Moonshine model from {}", dir.display()))?;
                Ok(Engine::Moonshine(m))
            }
        }
    }

    /// samples: 16 kHz mono f32 in [-1, 1].
    ///
    /// Long utterances are split into chunks. Moonshine rejects anything over
    /// 64s outright (`Audio duration must be between 0.1s and 64s`), so without
    /// this a user who talks for a minute loses the entire transcription rather
    /// than merely waiting longer for it.
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        let max = (MAX_CHUNK_SECS * SAMPLE_RATE as f32) as usize;
        if samples.len() <= max {
            return self.transcribe_one(samples);
        }

        let mut out: Vec<String> = Vec::new();
        let mut chunks = 0usize;
        let mut start = 0usize;
        while start < samples.len() {
            let end = split_point(samples, start, max);
            let text = self.transcribe_one(&samples[start..end])?;
            chunks += 1;
            if !text.is_empty() {
                out.push(text);
            }
            start = end;
        }
        // Report the real chunk count, and say so loudly when a chunk produced
        // nothing — an empty chunk means the utterance silently lost that whole
        // stretch of speech, which is not something to log as success.
        let empty = chunks - out.len();
        if empty > 0 {
            log::warn!(
                "{empty} of {chunks} chunks transcribed to nothing over {:.1}s of audio",
                samples.len() as f32 / SAMPLE_RATE as f32
            );
        } else {
            log::info!(
                "chunked {:.1}s of audio into {chunks} passes",
                samples.len() as f32 / SAMPLE_RATE as f32
            );
        }
        Ok(out.join(" "))
    }

    fn transcribe_one(&mut self, samples: &[f32]) -> Result<String> {
        let text = match self {
            Engine::Parakeet(m) => m
                .transcribe_with(samples, &ParakeetParams::default())
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .text,
            Engine::Moonshine(m) => m
                .transcribe_with(samples, &MoonshineParams::default())
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .text,
        };
        Ok(text.trim().to_string())
    }
}

/// Well under Moonshine's hard 64s ceiling, and short enough that a chunk's
/// inference stays comfortably sub-second on modest hardware.
pub const MAX_CHUNK_SECS: f32 = 40.0;
/// How far back from the chunk boundary we may cut to find a quiet moment.
const SEARCH_SECS: f32 = 6.0;
/// Window used to measure loudness when hunting for that quiet moment.
const RMS_WIN_MS: usize = 30;

/// End index for a chunk beginning at `start`, preferring the quietest point in
/// the last `SEARCH_SECS` of the window so we cut between words rather than
/// through one. Falls back to a hard cut when the audio has no quiet moment.
fn split_point(samples: &[f32], start: usize, max: usize) -> usize {
    let hard_end = (start + max).min(samples.len());
    if hard_end == samples.len() {
        return hard_end;
    }
    let search_len = (SEARCH_SECS * SAMPLE_RATE as f32) as usize;
    let search_start = hard_end.saturating_sub(search_len).max(start);
    let win = (RMS_WIN_MS * SAMPLE_RATE as usize) / 1000;
    if hard_end - search_start <= win {
        return hard_end;
    }

    let mut best = hard_end;
    let mut best_energy = f32::MAX;
    let mut i = search_start;
    while i + win <= hard_end {
        let energy: f32 = samples[i..i + win].iter().map(|s| s * s).sum();
        if energy < best_energy {
            best_energy = energy;
            best = i + win / 2;
        }
        i += win / 2;
    }
    best
}
