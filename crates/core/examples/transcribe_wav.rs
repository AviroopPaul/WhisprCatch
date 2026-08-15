//! Engine smoke test without audio-capture deps:
//! cargo run -p wc-core --no-default-features --example transcribe_wav -- <wav>

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let wav = PathBuf::from(std::env::args().nth(1).expect("usage: transcribe_wav <wav>"));
    // second arg picks the model, defaulting to the one the .deb ships with
    let model = wc_models::ModelId::parse(
        &std::env::args().nth(2).unwrap_or_else(|| "parakeet".into()),
    );
    let model_dir = wc_core::models_dir().join(model.spec().dir_name);

    let t0 = std::time::Instant::now();
    let mut engine = wc_core::engine::Engine::load(model, &model_dir)?;
    eprintln!("model loaded in {:.1}s", t0.elapsed().as_secs_f32());

    let samples = transcribe_rs::audio::read_wav_samples(&wav).map_err(|e| anyhow::anyhow!("{e}"))?;
    let audio_secs = samples.len() as f32 / 16_000.0;

    let t0 = std::time::Instant::now();
    let text = engine.transcribe(&samples)?;
    let dt = t0.elapsed().as_secs_f32();
    eprintln!("{audio_secs:.1}s audio → {dt:.2}s inference ({:.1}x realtime)", audio_secs / dt);
    println!("{text}");
    Ok(())
}
