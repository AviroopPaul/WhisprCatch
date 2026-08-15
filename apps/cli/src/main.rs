mod autostart;
mod config;
mod overlay;
mod settings_app;
mod shot;
mod stream;
mod theme;
mod wizard;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use wc_core::audio::Capture;
use wc_core::engine::Engine;
use wc_core::state::AppState;
use wc_hotkey::{PttEvent, PttKey};
use wc_inject::Injector;

use crate::stream::{join_delta, resume_at, split_words, Stream};

#[derive(Parser)]
#[command(name = "whisper-catch", version, about = "Local push-to-talk dictation")]
struct Cli {
    /// Model directory (overrides config; defaults to <data-dir>/whisper-catch/models/parakeet-tdt-0.6b-v2-int8)
    #[arg(long, global = true)]
    model: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

/// Launched with no subcommand (e.g. double-clicking the macOS app) → run the
/// push-to-talk daemon with defaults.
fn default_cmd() -> Cmd {
    Cmd::Ptt {
        key: None,
        print_only: false,
        no_tray: false,
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Transcribe a WAV file (engine smoke test)
    Transcribe { wav: PathBuf },
    /// Record N seconds from the mic, then transcribe
    Record {
        #[arg(long, default_value_t = 5)]
        seconds: u64,
    },
    /// Push-to-talk daemon: hold key, speak, release; text is typed at cursor
    Ptt {
        /// PTT key: rctrl, lctrl, ralt, lalt, super, f13, scrolllock (overrides config)
        #[arg(long)]
        key: Option<String>,
        /// Print transcripts to stdout instead of typing them
        #[arg(long)]
        print_only: bool,
        /// Run without the system tray icon
        #[arg(long)]
        no_tray: bool,
    },
    /// Open the settings & history window
    Settings {
        /// Tab to open: history (default) or settings
        #[arg(long)]
        tab: Option<String>,
    },
    /// Internal: floating recording indicator (spawned by the daemon)
    #[command(hide = true)]
    Overlay,
    /// Download the default model without starting the daemon
    DownloadModel,
    /// Print permission + model status (troubleshooting)
    Doctor,
    /// Internal: replay a WAV through the streaming loop to measure it
    #[command(hide = true)]
    SimulateStream {
        wav: PathBuf,
        /// Window cap in seconds; 0 means unbounded (the pre-window behaviour)
        #[arg(long, default_value_t = crate::stream::MAX_WINDOW_SECS)]
        window: f32,
    },
    /// Start whisper-catch automatically on login
    Autostart {
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let cfg = config::load()?;
    let cmd = cli.cmd.unwrap_or_else(default_cmd);

    // subcommands that don't need the engine
    match &cmd {
        Cmd::Settings { tab } => return settings_app::run(tab.clone()),
        Cmd::Overlay => return overlay::run(),
        Cmd::DownloadModel => {
            let model_id = wc_models::ModelId::parse(&cfg.model);
            let dir = model_id.spec().ensure(&wc_core::models_dir())?;
            log::info!("{} model ready at {}", model_id.slug(), dir.display());
            return Ok(());
        }
        Cmd::Doctor => {
            let model_id = wc_models::ModelId::parse(&cfg.model);
            let yn = |b: bool| if b { "yes" } else { "NO" };
            println!("WhisprCatch {}", env!("CARGO_PKG_VERSION"));
            println!("  config:        {}", config::config_path().display());
            println!("  models dir:    {}", wc_core::models_dir().display());
            println!("  model:         {} ({})", model_id.slug(), model_id.label());
            println!(
                "  model ready:   {}",
                yn(model_id.spec().is_complete(&wc_core::models_dir()))
            );
            println!(
                "  hotkey:        {} ({})",
                config::key_label(&cfg.key),
                cfg.key
            );
            #[cfg(target_os = "macos")]
            {
                println!("  Accessibility:    {}", yn(wc_hotkey::keyboard_accessible()));
                println!(
                    "  Input Monitoring: {}",
                    yn(wc_hotkey::input_monitoring_granted())
                );
                println!("  (Microphone is prompted automatically on first capture.)");
                println!(
                    "\nNote: after enabling a permission you must QUIT and REOPEN the app — \
                     macOS caches the grant per process."
                );
            }
            return Ok(());
        }
        Cmd::Autostart { enable, disable } => {
            if *disable {
                autostart::disable()?;
            } else {
                let _ = enable; // --enable is the default action
                autostart::enable()?;
            }
            return Ok(());
        }
        _ => {}
    }

    // Ptt is the desktop-launch entry point: single-instance guard and the
    // GUI setup wizard come before any console-style failure.
    if let Cmd::Ptt { .. } = &cmd {
        let _lock = match acquire_instance_lock() {
            Some(l) => l,
            None => {
                // already running — clicking the app icon should do something
                // useful, so open the settings window instead
                log::info!("daemon already running; opening settings");
                return settings_app::run(None);
            }
        };
        let model_id = wc_models::ModelId::parse(&cfg.model);
        if wizard::need_setup(model_id) && cli.model.is_none() && cfg.model_dir.is_none() {
            if gui_session() {
                match wizard::run(model_id, config::key_label(&cfg.key))? {
                    wizard::Outcome::Ready => {}
                    wizard::Outcome::Cancelled => return Ok(()),
                }
            } else if !wc_hotkey::keyboard_accessible() {
                anyhow::bail!(
                    "no access to input devices — run 'sudo usermod -aG input $USER' \
                     and re-login, or launch whisper-catch from your app menu to set up graphically"
                );
            }
        }
        // leak: hold the lock for the daemon's lifetime
        std::mem::forget(_lock);
    }

    let model_id = wc_models::ModelId::parse(&cfg.model);
    let model_dir = match cli.model.or_else(|| cfg.model_dir.clone()) {
        Some(dir) => dir, // explicit dir: user manages it, don't auto-download
        None => model_id
            .spec()
            .ensure(&wc_core::models_dir())
            .with_context(|| format!("fetching {} model", model_id.slug()))?,
    };

    log::info!("loading {} model from {}", model_id.slug(), model_dir.display());
    let t0 = std::time::Instant::now();
    let mut engine = Engine::load(model_id, &model_dir)?;
    log::info!("model loaded in {:.1}s", t0.elapsed().as_secs_f32());

    match cmd {
        Cmd::Transcribe { wav } => {
            let samples = transcribe_rs::audio::read_wav_samples(&wav)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .with_context(|| format!("reading {}", wav.display()))?;
            let t0 = std::time::Instant::now();
            let text = engine.transcribe(&samples)?;
            log::info!("inference took {:.2}s", t0.elapsed().as_secs_f32());
            println!("{text}");
        }
        Cmd::SimulateStream { wav, window } => return simulate_stream(&mut engine, &wav, window),
        Cmd::Record { seconds } => {
            let cap = Capture::open()?;
            cap.begin();
            eprintln!("recording {seconds}s — speak now...");
            std::thread::sleep(Duration::from_secs(seconds));
            let samples = cap.end()?;
            let t0 = std::time::Instant::now();
            let text = engine.transcribe(&samples)?;
            log::info!("inference took {:.2}s", t0.elapsed().as_secs_f32());
            println!("{text}");
        }
        Cmd::Ptt {
            key,
            print_only,
            no_tray,
        } => {
            let key_slug = key.as_deref().unwrap_or(&cfg.key).to_string();
            let key = PttKey::parse(&key_slug)?;
            let state = Arc::new(AppState::new());
            let tray_info = wc_tray::TrayInfo {
                model: model_id.label().to_string(),
                hotkey: config::key_label(&key_slug).to_string(),
            };

            #[cfg(target_os = "macos")]
            if no_tray {
                // Headless CLI use — run the loop on the main thread, no menu bar.
                run_ptt(state, engine, key, print_only, true, &cfg, tray_info)?;
            } else {
                // The macOS menu-bar tray must own the main thread, so the
                // dictation loop runs on a worker while the tray blocks here.
                let self_exe = std::env::current_exe().context("resolving own binary path")?;
                let worker_state = state.clone();
                let cfg2 = cfg.clone();
                let info2 = tray_info.clone();
                std::thread::spawn(move || {
                    if let Err(e) =
                        run_ptt(worker_state, engine, key, print_only, true, &cfg2, info2)
                    {
                        log::error!("dictation loop stopped: {e:#}");
                        if gui_session() {
                            notify("WhisprCatch stopped", &format!("{e:#}"));
                        }
                    }
                });
                wc_tray::run_main(state, self_exe, tray_info)?;
            }

            #[cfg(not(target_os = "macos"))]
            {
                let res = run_ptt(state, engine, key, print_only, no_tray, &cfg, tray_info);
                if let Err(e) = &res {
                    if gui_session() {
                        notify("WhisprCatch stopped", &format!("{e:#}"));
                        wizard::error_window(&format!("{e:#}"));
                    }
                }
                res?;
            }
        }
        Cmd::Settings { .. }
        | Cmd::Overlay
        | Cmd::DownloadModel
        | Cmd::Doctor
        | Cmd::Autostart { .. } => {
            unreachable!()
        }
    }
    Ok(())
}

/// No terminal attached — launched from the app menu / autostart.
fn gui_session() -> bool {
    use std::io::IsTerminal;
    !std::io::stderr().is_terminal()
}

fn notify(summary: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .icon("audio-input-microphone")
        .show();
}

/// Returns None when another daemon instance already holds the lock.
fn acquire_instance_lock() -> Option<std::fs::File> {
    use fs2::FileExt;
    let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    let path = dir.join("whisper-catch.lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&path)
        .ok()?;
    match f.try_lock_exclusive() {
        Ok(()) => Some(f),
        Err(_) => None,
    }
}

/// Close the mic this long after the last utterance — instant re-dictation
/// within the window, and the OS mic-in-use indicator clears soon after.
const MIC_IDLE_CLOSE: Duration = Duration::from_secs(10);
/// Rolling transcription cadence while the key is held. Word latency is
/// roughly two intervals (LocalAgreement needs consecutive passes to agree)
/// plus inference, so keep this tight — inference is only ~0.1-0.5s.
const STREAM_INTERVAL: Duration = Duration::from_millis(500);

struct OverlayProc(std::process::Child);

impl OverlayProc {
    /// `exe` is resolved once at daemon startup: after a package upgrade
    /// replaces the binary, current_exe() of the running daemon points at
    /// "… (deleted)" and every spawn would fail.
    fn spawn(exe: &std::path::Path) -> Option<Self> {
        match std::process::Command::new(exe)
            .arg("overlay")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => {
                log::info!("overlay spawned (pid {})", child.id());
                Some(Self(child))
            }
            Err(e) => {
                log::warn!("overlay spawn failed: {e}");
                None
            }
        }
    }

    fn transcribing(&mut self) {
        if let Some(stdin) = self.0.stdin.as_mut() {
            use std::io::Write;
            let _ = writeln!(stdin, "t");
        }
    }

    fn close(mut self) {
        drop(self.0.stdin.take()); // EOF → overlay exits
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            let _ = self.0.kill();
            let _ = self.0.wait();
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn run_ptt(
    state: Arc<AppState>,
    mut engine: Engine,
    key: PttKey,
    print_only: bool,
    no_tray: bool,
    cfg: &config::Config,
    tray_info: wc_tray::TrayInfo,
) -> Result<()> {
    let tray = if no_tray {
        None
    } else {
        match wc_tray::spawn(state.clone(), tray_info) {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("{e:#} — continuing without tray");
                None
            }
        }
    };
    let refresh = |t: &Option<wc_tray::TrayHandle>| {
        if let Some(t) = t {
            t.refresh();
        }
    };

    // resolve before any upgrade can replace the binary under us
    let self_exe = std::env::current_exe().context("resolving own binary path")?;

    let events = wc_hotkey::listen(key)?;
    let mut injector = if print_only {
        None
    } else {
        Some(Injector::new()?)
    };
    eprintln!("ready — hold {key:?} and speak, release to type. Ctrl-C to quit.");
    if gui_session() {
        notify(
            "WhisprCatch is running",
            &format!("Hold {key:?} and speak — release to type. Look for the mic in the top bar."),
        );
    }

    // mic is opened on demand and dropped after MIC_IDLE_CLOSE (no permanent
    // "mic in use" indicator in the top bar)
    let mut capture: Option<Capture> = None;
    let mut last_use = std::time::Instant::now();
    let mut armed = false;
    let mut overlay_proc: Option<OverlayProc> = None;

    // rolling-transcription state for the current utterance
    let mut stream = Stream::new();
    let mut modifier_lifted = false;
    let mut last_pass = std::time::Instant::now();

    // test hooks: SIGUSR1 = simulated press, SIGUSR2 = simulated release
    let sig_press = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sig_release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGUSR1, sig_press.clone());
    let _ = signal_hook::flag::register(signal_hook::consts::SIGUSR2, sig_release.clone());

    loop {
        let ev = if sig_press.swap(false, Ordering::Relaxed) {
            Ok(PttEvent::Pressed)
        } else if sig_release.swap(false, Ordering::Relaxed) {
            Ok(PttEvent::Released)
        } else {
            events.recv_timeout(Duration::from_millis(120))
        };
        match ev {
            Ok(PttEvent::Pressed) => {
                if !state.is_enabled() || armed {
                    continue;
                }
                if capture.is_none() {
                    match Capture::open() {
                        Ok(c) => capture = Some(c),
                        Err(e) => {
                            log::error!("mic open failed: {e:#}");
                            continue;
                        }
                    }
                }
                let cap = capture.as_ref().unwrap();
                cap.begin();
                armed = true;
                stream.reset();
                modifier_lifted = false;
                last_pass = std::time::Instant::now();
                log::info!("recording...");
                state.recording.store(true, Ordering::Relaxed);
                if cfg.overlay {
                    overlay_proc = OverlayProc::spawn(&self_exe);
                }
                refresh(&tray);
            }
            Ok(PttEvent::Released) => {
                if !armed {
                    continue;
                }
                armed = false;
                state.recording.store(false, Ordering::Relaxed);
                refresh(&tray);
                let cap = capture.as_ref().expect("armed without capture");
                last_use = std::time::Instant::now();

                let dur = cap.armed_secs();
                if dur < 0.3 && stream.committed().is_empty() {
                    cap.cancel();
                    if let Some(o) = overlay_proc.take() {
                        o.close();
                    }
                    log::info!("too short ({dur:.2}s), ignored");
                    continue;
                }
                if let Some(o) = overlay_proc.as_mut() {
                    o.transcribing();
                }
                let samples = match cap.end() {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("audio processing failed: {e:#}");
                        if let Some(o) = overlay_proc.take() {
                            o.close();
                        }
                        continue;
                    }
                };
                // An empty final transcript is the difference between "a bit
                // was lost" and "everything after the last streamed word was
                // lost", so record what actually went in.
                let level = (samples.iter().map(|s| s * s).sum::<f32>()
                    / samples.len().max(1) as f32)
                    .sqrt();
                log::debug!(
                    "final input: {} samples ({:.1}s), rms {level:.4}",
                    samples.len(),
                    samples.len() as f32 / wc_core::SAMPLE_RATE as f32
                );
                let t0 = std::time::Instant::now();
                let result = engine.transcribe(&samples);
                if let Some(o) = overlay_proc.take() {
                    o.close();
                }
                match result {
                    Ok(text) if text.is_empty() && stream.committed().is_empty() => {
                        log::info!("(empty transcript)")
                    }
                    Ok(text) => {
                        let infer_s = t0.elapsed().as_secs_f32();
                        log::info!("{dur:.1}s audio → {infer_s:.2}s inference (final)");
                        state.record_utterance(text.split_whitespace().count(), dur);
                        if cfg.history {
                            let entry = wc_core::history::Entry {
                                ts: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0),
                                dur_s: dur,
                                infer_s,
                                text: text.clone(),
                            };
                            if let Err(e) = wc_core::history::append(&entry) {
                                log::warn!("history write failed: {e:#}");
                            }
                        }
                        refresh(&tray);

                        let final_words = split_words(&text);
                        // words already typed by rolling passes stay put; type
                        // only what's left, aligned on the text rather than on a
                        // word count that the final pass may have shifted
                        let start = resume_at(stream.committed(), &final_words);
                        log::debug!(
                            "final: {} words, {} committed, resuming at {}",
                            final_words.len(),
                            stream.committed().len(),
                            start
                        );
                        if let Some(inj) = injector.as_mut() {
                            // let the user finish releasing the modifier so
                            // injected keys don't combine with it
                            std::thread::sleep(Duration::from_millis(150));
                            if start < final_words.len() {
                                let delta =
                                    join_delta(&final_words[start..], stream.nothing_typed());
                                if let Err(e) = inj.type_text(&delta) {
                                    log::error!("injection failed: {e:#}");
                                    println!("{text}");
                                }
                            }
                        } else {
                            println!("{text}");
                        }
                    }
                    Err(e) => log::error!("transcription failed: {e:#}"),
                }
                stream.reset();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // rolling transcription while the key is held
                if armed
                    && cfg.streaming
                    && injector.is_some()
                    && last_pass.elapsed() >= STREAM_INTERVAL
                {
                    last_pass = std::time::Instant::now();
                    if let Some(cap) = capture.as_ref() {
                        // A pass only ever transcribes a bounded window (see
                        // stream.rs), so its cost stops growing with the
                        // utterance and live typing continues for as long as the
                        // user talks.
                        let armed_samples = cap.armed_samples();
                        let rate = cap.device_rate();
                        if armed_samples as f32 / rate as f32 >= 0.5 {
                            if stream.maybe_slide(armed_samples, rate) {
                                log::debug!(
                                    "window slid to {:.1}s",
                                    stream.window_start() as f32 / rate as f32
                                );
                            }
                            let t0 = std::time::Instant::now();
                            match cap
                                .snapshot_from(stream.window_start())
                                .and_then(|snap| engine.transcribe(&snap))
                            {
                                Ok(text) => {
                                    let hyp = split_words(&text);
                                    let hyp_len = hyp.len();
                                    let delta = stream.advance(hyp);
                                    log::debug!(
                                        "pass: {:.2}s, window {:.1}s, {} words, +{}",
                                        t0.elapsed().as_secs_f32(),
                                        (armed_samples - stream.window_start()) as f32 / rate as f32,
                                        hyp_len,
                                        delta.len()
                                    );
                                    if !delta.is_empty() {
                                        let inj = injector.as_mut().unwrap();
                                        if !modifier_lifted {
                                            // fake-release the held PTT key at the
                                            // display-server level so our keystrokes
                                            // don't become modifier+letter shortcuts
                                            inj.lift_key(key.evdev_code());
                                            modifier_lifted = true;
                                        }
                                        let text = join_delta(&delta, stream.nothing_typed());
                                        if let Err(e) = inj.type_text(&text) {
                                            log::error!("streaming injection failed: {e:#}");
                                        } else {
                                            // debug, not info: this is the user's
                                            // speech — it should not reach logs
                                            // by default.
                                            log::debug!("streamed {text:?}");
                                            stream.mark_first_typed();
                                        }
                                    }
                                }
                                Err(e) => log::warn!("streaming pass failed: {e:#}"),
                            }
                        }
                    }
                }
                // release the mic after a quiet spell
                if !armed && capture.is_some() && last_use.elapsed() >= MIC_IDLE_CLOSE {
                    capture = None;
                    log::info!("mic released (idle)");
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

/// Replays a WAV through the real streaming state machine and reports what the
/// user would have seen. Exists so the windowing can be exercised end to end,
/// against real audio and the real model, without a microphone or a human.
///
/// Timing is modelled the way the daemon actually behaves: a pass is issued
/// every STREAM_INTERVAL, but a pass that overruns that interval means more
/// audio has accumulated by the time the next one starts.
fn simulate_stream(engine: &mut Engine, wav: &std::path::Path, window: f32) -> Result<()> {
    const RATE: u32 = wc_core::SAMPLE_RATE;
    let samples = transcribe_rs::audio::read_wav_samples(wav)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("reading {}", wav.display()))?;
    let total_secs = samples.len() as f32 / RATE as f32;

    // window <= 0 disables the cap, reproducing the unbounded behaviour
    let mut stream = if window > 0.0 {
        Stream::with_window(window, crate::stream::KEEP_TAIL_SECS)
    } else {
        Stream::with_window(f32::MAX, crate::stream::KEEP_TAIL_SECS)
    };
    let mut typed = String::new();
    let mut audio_pos = 0usize; // samples "captured" so far
    let mut passes = 0u32;
    let mut pass_secs: Vec<f32> = Vec::new();
    let mut max_window = 0f32;

    // Advance by a fixed interval rather than by measured pass duration: window
    // boundaries then fall in the same place every run, so the transcript is
    // reproducible and two builds can actually be compared. Pass cost is
    // reported separately, and the point of the window is that a pass now fits
    // inside the interval anyway.
    let interval = STREAM_INTERVAL.as_secs_f32();
    let mut elapsed = 0f32;
    while audio_pos < samples.len() {
        elapsed += interval;
        audio_pos = ((elapsed * RATE as f32) as usize).min(samples.len());
        if (audio_pos as f32 / RATE as f32) < 0.5 {
            continue;
        }
        stream.maybe_slide(audio_pos, RATE);
        let window = &samples[stream.window_start()..audio_pos];
        max_window = max_window.max(window.len() as f32 / RATE as f32);

        let t0 = std::time::Instant::now();
        let text = engine.transcribe(window)?;
        pass_secs.push(t0.elapsed().as_secs_f32());
        passes += 1;

        let delta = stream.advance(split_words(&text));
        if !delta.is_empty() {
            typed.push_str(&join_delta(&delta, stream.nothing_typed()));
            stream.mark_first_typed();
        }
    }

    // the release path: full (chunked) transcription, spliced onto what streamed
    let reference = engine.transcribe(&samples)?;
    let final_words = split_words(&reference);
    let start = resume_at(stream.committed(), &final_words);
    if start < final_words.len() {
        typed.push_str(&join_delta(&final_words[start..], stream.nothing_typed()));
    }

    let mean = pass_secs.iter().sum::<f32>() / pass_secs.len().max(1) as f32;
    let worst = pass_secs.iter().cloned().fold(0.0f32, f32::max);
    println!("audio_secs\t{total_secs:.1}");
    println!("passes\t{passes}");
    println!("pass_mean_s\t{mean:.3}");
    println!("pass_max_s\t{worst:.3}");
    println!("window_max_s\t{max_window:.1}");
    println!("streamed_words\t{}", stream.committed().len());
    println!("reference_words\t{}", final_words.len());
    println!("typed_words\t{}", typed.split_whitespace().count());
    println!("--- TYPED ---");
    println!("{typed}");
    println!("--- REFERENCE ---");
    println!("{reference}");
    Ok(())
}
