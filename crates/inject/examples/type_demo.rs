//! Manual smoke test for the injection backend.
//!
//! Run, then focus any text field within 3 seconds:
//!   cargo run -p wc-inject --example type_demo
//! Pass a custom string to type something else, and a second argument to force
//! an XKB layout ("gb", "us+dvorak") instead of detecting the session's.

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let text = std::env::args().nth(1).unwrap_or_else(|| {
        "The \"quick\" brown fox — email me@example.com, £9.99 (50% off)!".to_string()
    });
    let layout = std::env::args().nth(2);
    let mut injector = wc_inject::Injector::new(layout.as_deref())?;
    eprintln!("focus the target window — typing in 3s...");
    std::thread::sleep(std::time::Duration::from_secs(3));
    injector.type_text(&text)?;
    eprintln!("done");
    Ok(())
}
