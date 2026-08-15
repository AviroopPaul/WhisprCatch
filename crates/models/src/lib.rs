//! First-run model download: resumable (HTTP Range + .part file),
//! SHA-256 verified against hashes pinned at build time.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

pub struct FileSpec {
    /// Local filename the model loader expects under the model dir.
    pub name: &'static str,
    /// Path appended to `base_url` to fetch it. Often equal to `name`, but some
    /// repos lay files out differently (e.g. Moonshine's `onnx/…_int8.onnx`).
    pub url_path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub struct ModelSpec {
    pub dir_name: &'static str,
    pub base_url: &'static str,
    pub files: &'static [FileSpec],
}

/// A user-selectable speech model. Each variant maps to a download `ModelSpec`
/// and (in `wc-core`) to a transcribe-rs backend. Add a variant here + a
/// `ModelSpec` below + a match arm in `wc_core::engine` to introduce a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelId {
    /// NVIDIA Parakeet TDT 0.6B v2, int8 — most accurate, heavier (~1.5 GB RAM).
    Parakeet,
    /// Moonshine base, int8 — small & light (~0.4 GB RAM), great on an M1 Air.
    Moonshine,
}

impl ModelId {
    /// All models, in the order shown in the UI (default first).
    pub const ALL: [ModelId; 2] = [ModelId::Parakeet, ModelId::Moonshine];

    pub const fn default() -> Self {
        ModelId::Parakeet
    }

    /// Stable identifier written to the config file.
    pub const fn slug(self) -> &'static str {
        match self {
            ModelId::Parakeet => "parakeet",
            ModelId::Moonshine => "moonshine",
        }
    }

    /// Parses a config value; unknown/empty falls back to the default.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "moonshine" | "moonshine-base" | "small" | "light" => ModelId::Moonshine,
            "parakeet" | "parakeet-tdt-0.6b-v2" | "accurate" | "default" => ModelId::Parakeet,
            _ => ModelId::default(),
        }
    }

    /// The download spec for this model.
    pub const fn spec(self) -> &'static ModelSpec {
        match self {
            ModelId::Parakeet => &PARAKEET_V2_INT8,
            ModelId::Moonshine => &MOONSHINE_BASE_INT8,
        }
    }

    /// Short human label for the settings dropdown.
    pub const fn label(self) -> &'static str {
        match self {
            ModelId::Parakeet => "Parakeet 0.6B, accurate",
            ModelId::Moonshine => "Moonshine base, light",
        }
    }

    /// One-line description of the tradeoff.
    pub const fn blurb(self) -> &'static str {
        match self {
            ModelId::Parakeet => {
                "Best English accuracy. Heavier, best on 16 GB+ machines."
            }
            ModelId::Moonshine => {
                "Tiny and fast. Low memory, ideal for an 8 GB MacBook Air."
            }
        }
    }

    /// Rough resident-memory hint, for the UI.
    pub const fn ram_hint(self) -> &'static str {
        match self {
            ModelId::Parakeet => "~1.5 GB RAM",
            ModelId::Moonshine => "~0.4 GB RAM",
        }
    }

    /// Download size in whole megabytes.
    pub fn download_mb(self) -> u64 {
        (self.spec().total_size() + 500_000) / 1_000_000
    }
}

pub const PARAKEET_V2_INT8: ModelSpec = ModelSpec {
    dir_name: "parakeet-tdt-0.6b-v2-int8",
    base_url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main",
    files: &[
        FileSpec {
            name: "encoder-model.int8.onnx",
            url_path: "encoder-model.int8.onnx",
            size: 652_184_014,
            sha256: "3e0581fda6ab843888b51e56d7ee78b6d5bc3237ec113af1f732d1d5286aa155",
        },
        FileSpec {
            name: "decoder_joint-model.int8.onnx",
            url_path: "decoder_joint-model.int8.onnx",
            size: 8_998_286,
            sha256: "a449f49acd68979d418651dd2dcb737cc0f1bf0225e009e29ee326354edbf7d3",
        },
        FileSpec {
            name: "nemo128.onnx",
            url_path: "nemo128.onnx",
            size: 139_764,
            sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f",
        },
        FileSpec {
            name: "vocab.txt",
            url_path: "vocab.txt",
            size: 9_384,
            sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
        },
        FileSpec {
            name: "config.json",
            url_path: "config.json",
            size: 97,
            sha256: "666903c76b9798caf2c210afd4f6cd60b08a8dbf9800ec8d7a3bc0d2148ac466",
        },
    ],
};

/// Moonshine base, int8 ONNX (transformers.js export). ~64 MB, low RAM.
/// Files are renamed locally to what transcribe-rs's Moonshine loader expects
/// (`encoder_model.int8.onnx`, `decoder_model_merged.int8.onnx`, `tokenizer.json`).
pub const MOONSHINE_BASE_INT8: ModelSpec = ModelSpec {
    dir_name: "moonshine-base-int8",
    base_url: "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/main",
    files: &[
        FileSpec {
            name: "encoder_model.int8.onnx",
            url_path: "onnx/encoder_model_int8.onnx",
            size: 20_488_801,
            sha256: "4edb38db96b52ddee5a3b25d2211bb6394f9a9a2d95f67bce0e8c861da018a4d",
        },
        FileSpec {
            name: "decoder_model_merged.int8.onnx",
            url_path: "onnx/decoder_model_merged_int8.onnx",
            size: 42_427_261,
            sha256: "45febab0347ffbe1326459e092ae20a2cfda10ca33ab018b3c299247284de61b",
        },
        FileSpec {
            name: "tokenizer.json",
            url_path: "tokenizer.json",
            size: 3_761_754,
            sha256: "7b913404bdd039af4756783218af4440bc07fb7d6d8258d677e34f95b3ec416f",
        },
    ],
};

impl ModelSpec {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// True when every file is present with the right size.
    pub fn is_complete(&self, models_root: &Path) -> bool {
        let dir = models_root.join(self.dir_name);
        self.files
            .iter()
            .all(|f| dir.join(f.name).metadata().map(|m| m.len()).ok() == Some(f.size))
    }

    /// Ensures all model files exist under `models_root/<dir_name>`,
    /// downloading whatever is missing. Returns the model directory.
    pub fn ensure(&self, models_root: &Path) -> Result<PathBuf> {
        self.ensure_with(models_root, &|_, _, _| {})
    }

    /// Like `ensure`, reporting progress as (current file, bytes done
    /// across all files, total bytes across all files).
    pub fn ensure_with(
        &self,
        models_root: &Path,
        progress: &(dyn Fn(&str, u64, u64) + Sync),
    ) -> Result<PathBuf> {
        let dir = models_root.join(self.dir_name);
        std::fs::create_dir_all(&dir)?;
        let total = self.total_size();
        let mut done_before = 0u64;
        for f in self.files {
            let dest = dir.join(f.name);
            // size check only for existing files — full hash verify happens on
            // download; a hash pass over 650MB at every startup isn't worth it
            if dest.metadata().map(|m| m.len()).ok() == Some(f.size) {
                done_before += f.size;
                progress(f.name, done_before, total);
                continue;
            }
            let base = done_before;
            download(&format!("{}/{}", self.base_url, f.url_path), &dest, f, &|d| {
                progress(f.name, base + d, total)
            })?;
            done_before += f.size;
        }
        Ok(dir)
    }
}

fn download(
    url: &str,
    dest: &Path,
    spec: &FileSpec,
    progress: &(dyn Fn(u64) + Sync),
) -> Result<()> {
    let part = dest.with_extension(
        dest.extension()
            .map(|e| format!("{}.part", e.to_string_lossy()))
            .unwrap_or_else(|| "part".into()),
    );

    // resume: hash the existing partial file so the final digest stays valid
    let mut hasher = Sha256::new();
    let mut offset = 0u64;
    if let Ok(existing) = std::fs::read(&part) {
        if (existing.len() as u64) < spec.size {
            hasher.update(&existing);
            offset = existing.len() as u64;
        } else {
            std::fs::remove_file(&part).ok();
        }
    }

    log::info!(
        "downloading {} ({:.0} MB){}",
        spec.name,
        spec.size as f64 / 1e6,
        if offset > 0 {
            format!(", resuming at {:.0} MB", offset as f64 / 1e6)
        } else {
            String::new()
        }
    );

    let mut req = ureq::get(url);
    if offset > 0 {
        req = req.header("Range", &format!("bytes={offset}-"));
    }
    let mut resp = req.call().with_context(|| format!("GET {url}"))?;
    if offset > 0 && resp.status() != 206 {
        // server ignored the Range header; start over
        log::warn!("server ignored resume request, restarting {}", spec.name);
        hasher = Sha256::new();
        offset = 0;
        std::fs::remove_file(&part).ok();
    }

    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(offset > 0)
        .write(true)
        .truncate(offset == 0)
        .open(&part)
        .with_context(|| format!("opening {}", part.display()))?;

    let mut reader = resp.body_mut().as_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = offset;
    let mut last_pct = 0;
    loop {
        let n = reader.read(&mut buf).context("reading response body")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(done);
        let pct = (done * 100 / spec.size) as u32;
        if pct >= last_pct + 10 {
            log::info!("  {} — {pct}%", spec.name);
            last_pct = pct;
        }
    }
    out.flush()?;
    drop(out);

    if done != spec.size {
        bail!(
            "{}: incomplete download ({done} of {} bytes). Rerun to resume",
            spec.name,
            spec.size
        );
    }
    let digest = hex::encode(hasher.finalize());
    if digest != spec.sha256 {
        std::fs::remove_file(&part).ok();
        bail!("{}: checksum mismatch (got {digest}). Corrupted download removed, rerun", spec.name);
    }
    std::fs::rename(&part, dest)?;
    log::info!("  {} — verified", spec.name);
    Ok(())
}
