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

use crate::stream::{
    begin_utterance, finish_utterance, join_delta, plan_release, split_words, stream_delta,
    Release, ReleaseInput, Stream, TextSink,
};

/// The dictation loop's view of the injector. The trait is ours, so this is the
/// one place the daemon touches `wc_inject` directly and the sequencing around
/// it can be tested against a fake (`stream.rs`).
impl TextSink for Injector {
    fn type_text(&mut self, text: &str) -> Result<()> {
        Injector::type_text(self, text)
    }
    fn replace_last(&mut self, n_chars: usize, text: &str) -> Result<()> {
        Injector::replace_last(self, n_chars, text)
    }
    fn forget_typed(&mut self) {
        Injector::forget_typed(self)
    }
    fn replaceable_chars(&self) -> usize {
        Injector::replaceable_chars(self)
    }
}

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
    /// Internal: run the first-run setup wizard on its own (design + QA)
    #[command(hide = true)]
    Wizard,
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
        Cmd::Wizard => {
            let model_id = wc_models::ModelId::parse(&cfg.model);
            wizard::run(model_id, config::key_label(&cfg.key))?;
            return Ok(());
        }
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
                    "\nNote: after enabling a permission you must QUIT and REOPEN the app. \
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
                    "no access to input devices. Run 'sudo usermod -aG input $USER' \
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
        // Engine smoke tests: Transcribe and Record below print the model's own
        // output, deliberately unpolished, so a cleanup bug (#36) can be told
        // apart from a model one.
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
            eprintln!("recording {seconds}s, speak now...");
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
        | Cmd::Wizard
        | Cmd::DownloadModel
        | Cmd::Doctor
        | Cmd::Autostart { .. } => {
            unreachable!()
        }
    }
    Ok(())
}

/// The transcription→injection seam (#40): run the polish chain over the raw
/// transcript and decide what history should keep.
///
/// Returns `(text_to_type, raw_for_history)`. `raw` is `Some` only when polish
/// actually changed something — there is no point storing two copies of an
/// untouched transcript, and undo (#42) has nothing to undo. Kept as a free
/// function so the seam is testable without a model or a microphone.
fn finish(polish: &wc_text::Polish, raw: String) -> (String, Option<String>) {
    let text = polish.apply(&raw);
    if text == raw {
        return (text, None);
    }
    // debug, not info: this is the user's speech — it should not reach logs
    // by default.
    log::debug!("polish: {raw:?} -> {text:?}");
    (text, Some(raw))
}

/// Whether the release pass may take back what live typing put on screen.
///
/// Two independent conditions, both required:
///
/// * **The sink can release the held push-to-talk modifier** (#77). Where it
///   cannot, Backspace arrives as Ctrl+Backspace, which is delete-*word*.
/// * **The sink has a pasteboard** (#68). This is a cost gate, not a
///   correctness one, and it is why the replace path is off for everyone in
///   v0.5. A realistic 60-second dictation with filler removal and an early
///   "um" is 731 backspaces plus 710 retyped characters: 1462 unpaced CGEvents
///   on macOS, 731 XTEST round trips on Linux, nothing confirming any of them
///   landed, and a single dropped backspace splices the new text into the
///   middle of the old. `PASTE_THRESHOLD` in `wc_inject` already says a few
///   hundred synthesised keystrokes is more than apps reliably take — that
///   judgement applies to a wipe-and-retype at least as much as to a long
///   type. With filler removal shipped, an early "um" is the *modal* case, not
///   the rare one.
///
/// No backend sets `paste` yet, so this is false everywhere today and the
/// streaming/cleanup combination resolves by not typing live at all. When #68
/// lands the replace path switches on with no change here.
fn replace_available(caps: wc_inject::Capabilities) -> bool {
    caps.can_lift_modifiers && caps.paste
}

/// Whether to type words live while the key is held.
///
/// Live typing puts the model's raw words on screen as they settle, and the
/// release pass then replaces them with the cleaned-up transcript
/// (`stream::plan_release`). That only works where the text can be taken back
/// again — see [`replace_available`].
///
/// So when cleanup can rewrite words and they could not be corrected
/// afterwards, we do not type them in the first place. The broken combination
/// is made *unreachable* rather than warned about: with no live typing there is
/// nothing on screen for the cleanup pass to disagree with, and the finished
/// text is typed once on release exactly as with streaming off.
fn live_typing_enabled(streaming: bool, rewriting: bool, can_replace: bool) -> bool {
    streaming && (!rewriting || can_replace)
}

/// Whether the cleanup chain can rewrite words that live typing already typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupRewrites(pub bool);

/// Whether the release pass could take those words back again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceAvailable(pub bool);

/// What the Settings "Live typing" row says under the toggle.
///
/// #50's done-when is that the chosen behaviour is documented where the user
/// can see it, so this is a decision and not a decoration — hence a function
/// with its own test rather than a `match` inlined into a paint call.
///
/// The two arguments are newtypes and not bools on purpose. A unit test can
/// pin what each combination *says*, but nothing in this workspace can reach
/// the call site inside an egui render to catch the arguments being passed the
/// wrong way round — so the types make that a compile error instead of
/// something a test might have caught.
pub fn live_typing_description(
    rewrites: CleanupRewrites,
    replace: ReplaceAvailable,
) -> &'static str {
    match (rewrites.0, replace.0) {
        (false, _) => "Words appear while you speak",
        (true, true) => "Words appear while you speak, then text cleanup corrects them when you release",
        (true, false) => "Paused while text cleanup is on: cleaned-up words can differ from what you said",
    }
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

    // The deterministic cleanup chain (#40). Built once: it is pure, so the
    // same chain serves every utterance. Empty unless the user enabled a
    // transform, in which case `apply` is the identity function.
    let polish = wc_text::Polish::from_config(&cfg.polish);
    if polish.is_empty() {
        log::debug!("text polish: nothing enabled");
    } else {
        log::info!("text polish: {}", polish.names().join(" -> "));
    }

    let listener = wc_hotkey::listen(key)?;
    let events = &listener.events;
    let mut injector = if print_only {
        None
    } else {
        Some(Injector::new()?)
    };

    // Streaming types the model's raw words as they settle; the cleanup chain
    // only runs on the finished transcript, and none of its six transforms is
    // prefix-stable — even a substitution rewrites text already on screen when
    // its trigger phrase straddles the streaming boundary. The release pass
    // reconciles the two by replacing what was streamed (`plan_release`), which
    // needs an injector that can take text back.
    let can_replace = injector
        .as_ref()
        .is_some_and(|i| replace_available(wc_inject::capabilities_for(i.can_replace())));
    let live_typing = injector.is_some()
        && live_typing_enabled(cfg.streaming, polish.has_rewriting_transforms(), can_replace);
    if cfg.streaming && injector.is_some() {
        if live_typing && polish.has_rewriting_transforms() {
            log::info!(
                "live typing shows the model's own words; text cleanup replaces them \
                 when you release the key"
            );
        } else if !live_typing {
            log::warn!(
                "live typing is off this session: text cleanup can rewrite words that \
                 have already been typed, and this session cannot take them back \
                 safely. The finished text is typed once on release instead."
            );
        }
    }

    eprintln!("ready. Hold {key:?} and speak, release to type. Ctrl-C to quit.");
    if gui_session() {
        notify(
            "WhisprCatch is running",
            &format!("Hold {key:?} and speak, release to type. Look for the mic in the top bar."),
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
    // Other keys pressed as of the last press, so the release can tell whether
    // the user typed something of their own while holding the hotkey. Always 0
    // on macOS with a modifier hotkey — see `Listener::other_key_presses`.
    let mut keys_at_press = 0u64;
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
                match injector.as_mut() {
                    Some(inj) => begin_utterance(inj, &mut stream),
                    None => stream.reset(),
                }
                keys_at_press = listener.other_key_presses();
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
                    Ok(raw) => {
                        let infer_s = t0.elapsed().as_secs_f32();
                        log::info!("{dur:.1}s audio → {infer_s:.2}s inference (final)");
                        // The seam (#40): every deterministic cleanup pass runs
                        // here, on the finished transcript, before history and
                        // before a single character is typed.
                        let (text, polished_from) = finish(&polish, raw);
                        // Whether cleanup rewrote the model's words, which is
                        // what decides between appending to the streamed text
                        // and replacing it. Taken before `polished_from` is
                        // moved into the history entry.
                        let rewritten = polished_from.is_some();
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
                                raw: polished_from,
                            };
                            if let Err(e) = wc_core::history::append(&entry) {
                                log::warn!("history write failed: {e:#}");
                            }
                        }
                        refresh(&tray);

                        if let Some(inj) = injector.as_mut() {
                            // Somebody else's key went down while the hotkey
                            // was held, so the cursor is not where we left it.
                            let user_typed = listener.other_key_presses() != keys_at_press;
                            if user_typed {
                                log::debug!(
                                    "a key of the user's own went down while the hotkey \
                                     was held; the release will not take text back"
                                );
                            }
                            // let the user finish releasing the modifier so
                            // injected keys don't combine with it. This matters
                            // more for a replace than for a type: Ctrl+
                            // Backspace is delete-word.
                            std::thread::sleep(Duration::from_millis(150));
                            // Reconcile what the streaming passes typed with
                            // the finished transcript (#50). Every decision is
                            // in `plan_release`; this only carries it out.
                            let (release, sent) = finish_utterance(
                                inj,
                                &stream,
                                &text,
                                rewritten,
                                can_replace,
                                user_typed,
                            );
                            log::debug!(
                                "release: {} streamed chars, {} committed words -> {}",
                                stream.typed().chars().count(),
                                stream.committed().len(),
                                match &release {
                                    Release::Nothing => "nothing".to_string(),
                                    Release::Append(t) =>
                                        format!("append {} chars", t.chars().count()),
                                    Release::Replace { take_back, text } => format!(
                                        "replace {take_back} chars with {} chars",
                                        text.chars().count()
                                    ),
                                }
                            );
                            if let Err(e) = sent {
                                log::error!("injection failed: {e:#}");
                                println!("{text}");
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
                if armed && live_typing && last_pass.elapsed() >= STREAM_INTERVAL {
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
                                        // Types it and notes it only if it
                                        // landed — see `stream_delta`.
                                        let sent = stream_delta(inj, &mut stream, &delta);
                                        if !sent.is_empty() {
                                            // debug, not info: this is the user's
                                            // speech — it should not reach logs
                                            // by default.
                                            log::debug!("streamed {sent:?}");
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
            stream.mark_typed(&join_delta(&delta, stream.nothing_typed()));
        }
    }

    // The release path, through the same planner the daemon uses. No polish
    // chain here — the harness measures the streaming machinery, so `rewritten`
    // is false and the plan is the append the loop has always produced.
    let reference = engine.transcribe(&samples)?;
    let mut typed = stream.typed().to_string();
    match plan_release(ReleaseInput {
        streamed: stream.typed(),
        committed: stream.committed(),
        final_text: &reference,
        rewritten: false,
        replaceable_chars: stream.typed().chars().count(),
        can_replace: true,
        user_typed: false,
    }) {
        Release::Nothing => {}
        Release::Append(t) => typed.push_str(&t),
        Release::Replace { take_back, text } => {
            let keep = typed.chars().count().saturating_sub(take_back);
            typed = typed.chars().take(keep).chain(text.chars()).collect();
        }
    }
    let final_words = split_words(&reference);

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

#[cfg(test)]
mod tests {
    use super::*;
    use wc_text::{BoxedTransform, Polish, PolishConfig, Transform};

    /// A transform that actually changes text, standing in for the real ones
    /// until #43-#48 land. Nothing in `wc-text` changes a byte yet, so this is
    /// the only way to exercise the "polish changed it" branch of the seam.
    struct Shout;
    impl Transform for Shout {
        fn name(&self) -> &'static str {
            "shout"
        }
        fn apply(&self, text: &str) -> String {
            text.to_uppercase()
        }
        fn prefix_stable(&self) -> bool {
            true
        }
    }

    fn shouty() -> Polish {
        Polish::from_transforms(vec![Box::new(Shout) as BoxedTransform])
    }

    /// The promise this whole issue makes: with nothing enabled a dictation
    /// round-trip is byte-identical to v0.4.0, and history gains nothing.
    #[test]
    fn a_default_chain_changes_nothing_and_stores_no_raw() {
        let polish = Polish::from_config(&PolishConfig::default());
        for raw in [
            "",
            "hello world",
            "Um, I mean, twenty five percent.",
            "naïve café 👩‍💻 🚀",
            "  spaced  out  \n",
        ] {
            let (text, stored) = finish(&polish, raw.to_string());
            assert_eq!(text, raw, "polish changed {raw:?}");
            assert_eq!(stored, None, "stored a raw copy for unchanged {raw:?}");
        }
    }

    #[test]
    fn a_changing_chain_types_the_polished_text_and_keeps_the_raw() {
        let (text, stored) = finish(&shouty(), "hello world".to_string());
        assert_eq!(text, "HELLO WORLD");
        assert_eq!(stored.as_deref(), Some("hello world"));
    }

    /// A transform that happens to be a no-op on this input must not fill
    /// history with a duplicate of the text sitting next to it.
    #[test]
    fn no_raw_is_stored_when_the_chain_makes_no_difference() {
        let (text, stored) = finish(&shouty(), "ALREADY SHOUTING".to_string());
        assert_eq!(text, "ALREADY SHOUTING");
        assert_eq!(stored, None);
    }

    /// What actually lands on disk, end to end: an unpolished utterance writes
    /// the exact line shape v0.4.0 wrote, and a polished one adds `raw`.
    #[test]
    fn history_lines_match_the_old_format_until_polish_changes_something() {
        let line = |polish: &Polish, raw: &str| {
            let (text, stored) = finish(polish, raw.to_string());
            serde_json::to_string(&wc_core::history::Entry {
                ts: 1_754_000_000,
                dur_s: 2.5,
                infer_s: 0.31,
                text,
                raw: stored,
            })
            .unwrap()
        };
        assert_eq!(
            line(
                &Polish::from_config(&PolishConfig::default()),
                "hello world"
            ),
            r#"{"ts":1754000000,"dur_s":2.5,"infer_s":0.31,"text":"hello world"}"#
        );
        assert_eq!(
            line(&shouty(), "hello world"),
            r#"{"ts":1754000000,"dur_s":2.5,"infer_s":0.31,"text":"HELLO WORLD","raw":"hello world"}"#
        );
    }

    /// The stats the tray shows count what the user actually got, not what the
    /// model said — filler removal should lower the word count, not keep it.
    #[test]
    fn word_count_is_taken_from_the_polished_text() {
        struct DropLast;
        impl Transform for DropLast {
            fn name(&self) -> &'static str {
                "drop_last"
            }
            fn apply(&self, text: &str) -> String {
                let mut w: Vec<&str> = text.split_whitespace().collect();
                w.pop();
                w.join(" ")
            }
            fn prefix_stable(&self) -> bool {
                false
            }
        }
        let polish = Polish::from_transforms(vec![Box::new(DropLast) as BoxedTransform]);
        let (text, _) = finish(&polish, "one two three um".to_string());
        assert_eq!(text.split_whitespace().count(), 3);
    }

    /// The words already typed by streaming passes are reconciled against the
    /// final transcript. With nothing enabled, that is still the append it
    /// always was.
    #[test]
    fn polished_text_still_splices_onto_what_streaming_typed() {
        let polish = Polish::from_config(&PolishConfig::default());
        let raw = "the whole emotional spectrum drama".to_string();
        let (text, from) = finish(&polish, raw);
        let streamed = "the whole emotional spectrum";
        assert_eq!(
            plan_release(ReleaseInput {
                streamed,
                committed: &split_words(streamed),
                final_text: &text,
                rewritten: from.is_some(),
                replaceable_chars: streamed.chars().count(),
                can_replace: true,
                user_typed: false,
            }),
            Release::Append(" drama".into())
        );
    }

    // ---- the seam and the screen, end to end (#50) --------------------------

    /// The **real** filler-removal chain, configured the way the manual
    /// verification steps say to configure it.
    ///
    /// Deliberately not a hand-written stand-in. The first version of this test
    /// used one, and the recipe in the PR body (`enabled = true` alone) turned
    /// out to be a no-op against the shipped transform — `enabled` and `level`
    /// are two decisions there, so nothing was ever rewritten and the whole
    /// destructive path went unexercised. Driving the real transform is what
    /// keeps the documented recipe honest.
    fn filler_removal() -> Polish {
        Polish::from_config(&PolishConfig {
            fillers: wc_text::FillersConfig {
                enabled: true,
                level: wc_text::fillers::FillerLevel::Light,
            },
            ..PolishConfig::default()
        })
    }

    /// The screen a `Release` leaves behind, through the **real** injector
    /// planner rather than a model of it. See `stream.rs`'s copy for why that
    /// distinction cost a blocking review finding.
    fn screen_after(before: &str, release: &Release) -> String {
        use wc_inject::plan::{PlanOpts, Typed};
        match release {
            Release::Nothing => before.to_string(),
            Release::Append(t) => format!("{before}{t}"),
            Release::Replace { take_back, text } => {
                let mut record = Typed::new();
                record.record(before);
                record
                    .plan_replace(*take_back, text, PlanOpts::typing_only())
                    .simulate(before)
            }
        }
    }

    /// Drive one utterance through the real seam: the streaming passes type the
    /// model's raw words, the release pass polishes and reconciles. Returns
    /// what the user is left looking at.
    fn dictate(polish: &Polish, streamed: &str, heard: &str) -> String {
        let (text, from) = finish(polish, heard.to_string());
        let release = plan_release(ReleaseInput {
            streamed,
            committed: &split_words(streamed),
            final_text: &text,
            rewritten: from.is_some(),
            replaceable_chars: streamed.chars().count(),
            can_replace: true,
            user_typed: false,
        });
        screen_after(streamed, &release)
    }

    /// **The bug in #50, at the seam that has it**, driven by the shipping
    /// filler-removal transform. Streaming typed the fillers because that is
    /// what the model said; the finished transcript has them removed. What the
    /// user ends up with must be the finished transcript, not the fillers plus
    /// a tail.
    #[test]
    fn filler_removal_under_live_typing_leaves_the_polished_text_on_screen() {
        let polish = filler_removal();
        let heard = "So um I think uh we should ship it";
        // the streaming passes run two words behind, as the guard makes them
        let streamed = "So um I think uh we should";

        // The transform really does rewrite this, so `rewritten` is really true
        // — the check the old stand-in could not make.
        let (finished, from) = finish(&polish, heard.to_string());
        assert!(from.is_some(), "the recipe did not rewrite anything");
        assert_eq!(finished, "So I think we should ship it");

        assert_eq!(dictate(&polish, streamed, heard), finished);

        // The same utterance with cleanup off is untouched, fillers and all —
        // so the assertion above is about the cleanup pass and not about some
        // change to streaming itself.
        let off = Polish::from_config(&PolishConfig::default());
        assert_eq!(dictate(&off, streamed, heard), heard);
    }

    /// `enabled` alone is not the recipe: filler removal takes two decisions,
    /// and a config with only the first changes nothing at all. This is the
    /// mistake the PR's manual steps shipped with.
    #[test]
    fn enabling_filler_removal_without_a_level_rewrites_nothing() {
        let half_on = Polish::from_config(&PolishConfig {
            fillers: wc_text::FillersConfig {
                enabled: true,
                ..Default::default()
            },
            ..PolishConfig::default()
        });
        let heard = "So um I think uh we should ship it";
        let (text, from) = finish(&half_on, heard.to_string());
        assert_eq!(text, heard);
        assert!(
            from.is_none(),
            "a level-less config must not reach the replace path"
        );
    }

    #[test]
    fn a_default_chain_leaves_the_release_path_byte_identical() {
        let polish = Polish::from_config(&PolishConfig::default());
        for (streamed, heard) in [
            ("", "hello world"),
            ("hello", "hello world"),
            ("the quick brown", "the quick brown fox jumps"),
            ("all of it", "all of it"),
        ] {
            assert_eq!(
                dictate(&polish, streamed, heard),
                heard,
                "streamed {streamed:?}"
            );
        }
    }

    /// The v0.5 shipping answer: no backend can paste, so the replace path is
    /// off for everyone and the streaming/cleanup combination resolves by not
    /// typing live at all.
    #[test]
    fn the_replace_path_is_off_until_a_pasteboard_exists() {
        for can_lift in [true, false] {
            assert!(
                !replace_available(wc_inject::capabilities_for(can_lift)),
                "the wipe-and-retype is not affordable without #68 (can_lift={can_lift})"
            );
        }
        // Both halves are required, and the paste half is the one missing.
        assert!(!replace_available(wc_inject::Capabilities {
            paste: true,
            can_lift_modifiers: false
        }));
        assert!(replace_available(wc_inject::Capabilities {
            paste: true,
            can_lift_modifiers: true
        }));
    }

    /// The description under the Settings toggle is #50's other done-when, so
    /// it is chosen by a function rather than assembled inline where the arms
    /// could be swapped unnoticed.
    #[test]
    fn settings_says_what_live_typing_will_actually_do() {
        let desc = |cleanup, replace| {
            live_typing_description(CleanupRewrites(cleanup), ReplaceAvailable(replace))
        };
        // Cleanup off: unchanged, and the display server is irrelevant.
        for replace in [true, false] {
            assert_eq!(desc(false, replace), "Words appear while you speak");
        }
        // Cleanup on and correctable: say that it will be corrected.
        assert!(desc(true, true).contains("cleanup corrects them"));
        // Cleanup on and not correctable — the v0.5 state: say it is paused.
        assert!(desc(true, false).starts_with("Paused"));
        // The three arms say different things, so a mix-up cannot hide behind
        // two of them reading the same.
        let mut all = [desc(false, false), desc(true, true), desc(true, false)];
        all.sort_unstable();
        all.windows(2)
            .for_each(|p| assert_ne!(p[0], p[1], "two arms give the same text"));

        // And what Settings shows matches what the daemon will do. This is the
        // half that matters: a description that disagrees with the behaviour is
        // worse than no description.
        for cleanup in [true, false] {
            for replace in [true, false] {
                assert_eq!(
                    desc(cleanup, replace).starts_with("Paused"),
                    !live_typing_enabled(true, cleanup, replace),
                    "Settings and the daemon disagree at ({cleanup}, {replace})"
                );
            }
        }
    }

    /// Passing the two facts the wrong way round is a **compile** error, not a
    /// test failure, because no test in this workspace can reach the call site
    /// inside an egui paint. Kept as a documented compile-fail rather than a
    /// `#[test]`: there is no stable way to assert non-compilation without a
    /// `trybuild`-style dependency, and this milestone does not add one.
    ///
    /// ```compile_fail
    /// # use whisper_catch::{live_typing_description, CleanupRewrites, ReplaceAvailable};
    /// live_typing_description(ReplaceAvailable(true), CleanupRewrites(true));
    /// ```
    #[allow(dead_code)]
    fn the_description_arguments_cannot_be_swapped() {}

    /// Live typing is only offered where the release pass could correct it. The
    /// combination the issue calls unreachable is unreachable here, rather than
    /// warned about and left switched on.
    #[test]
    fn live_typing_is_refused_where_the_text_could_not_be_taken_back() {
        // streaming off: never live, whatever else is true
        assert!(!live_typing_enabled(false, false, true));
        assert!(!live_typing_enabled(false, true, true));

        // nothing rewrites: live typing is unaffected by the display server,
        // because there is never anything to take back
        assert!(live_typing_enabled(true, false, true));
        assert!(live_typing_enabled(true, false, false));

        // cleanup can rewrite: live typing only where a replace is possible
        assert!(live_typing_enabled(true, true, true));
        assert!(
            !live_typing_enabled(true, true, false),
            "streamed raw words that could never be corrected — this is the #50 bug"
        );
    }

    /// The gate is driven by `has_rewriting_transforms`, and every enabled
    /// transform answers true — including a chain of one. A user who turns on
    /// nothing but a custom dictionary is covered.
    #[test]
    fn any_single_enabled_transform_engages_the_gate() {
        assert!(filler_removal().has_rewriting_transforms());
        assert!(!Polish::from_config(&PolishConfig::default()).has_rewriting_transforms());
    }
}
