//! Manual harness for the half of `replace_last` that CI cannot reach.
//!
//! The counting is unit tested to death with no display server. What no test
//! can check is whether a synthetic Backspace actually deletes one grapheme
//! cluster *in the app you are looking at*, so this drives the real injector
//! against a real window and prints what should happen next to what does.
//!
//! ```text
//! cargo run -p wc-inject --example replace_demo
//! ```
//!
//! Focus a text field before the countdown ends. Run it once per app you care
//! about: a browser textarea, a native editor, an Electron app (Slack, VS
//! Code) and a terminal — they do not agree about what Backspace deletes, and
//! the emoji case is where they diverge.

use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;
use wc_inject::Injector;

/// One person plus one person plus two children: seven chars, one cluster, and
/// one Backspace press in any app that follows UAX #29.
const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";

fn main() -> Result<()> {
    let mut injector = Injector::new()?;

    print!("Focus a text field. Starting in ");
    for n in (1..=5).rev() {
        print!("{n}... ");
        std::io::stdout().flush().ok();
        sleep(Duration::from_secs(1));
    }
    println!();

    // 1. The plain case: only the changed tail moves.
    step(
        &mut injector,
        "plain replace",
        "hello world",
        5,
        "there",
        "hello there, typed without the leading `hello ` flickering",
    )?;

    // 2. Multibyte: five clusters, fifteen bytes.
    step(
        &mut injector,
        "CJK",
        "你好世界",
        2,
        "朋友",
        "你好朋友 — two presses, not six",
    )?;

    // 3. The one that separates a char count from a cluster count. If the app
    //    deletes per code point you will see the family emoji lose members one
    //    at a time instead of vanishing whole; that is the app's segmentation,
    //    and it is what this harness exists to find out.
    step(
        &mut injector,
        "emoji ZWJ sequence",
        &format!("hi {FAMILY}"),
        7,
        "everyone",
        "hi everyone — the whole family goes on one press",
    )?;

    // 4. Combining mark: the accent cannot be deleted on its own.
    step(
        &mut injector,
        "combining mark",
        "cafe\u{301}",
        1,
        "x",
        "cafex — the é is removed whole and the e retyped",
    )?;

    // 5. Long: 400 backspaces in a burst. Watch for dropped presses — this is
    //    where an app's event queue gives up, and there is no pacing yet.
    let long = "long ".repeat(80);
    step(
        &mut injector,
        "400 chars",
        &long,
        long.chars().count(),
        "gone",
        "gone — nothing left of the long run, no leftovers",
    )?;

    // 6. Nothing of the user's own text may be touched, however much is asked
    //    for. Type something yourself first if you want to watch this one.
    step(
        &mut injector,
        "over-large count",
        "ours",
        9_999,
        "safe",
        "safe — and anything you typed before it still there",
    )?;

    println!(
        "\nNow the modifier case, which is the one that damages text.\n\
         Hold Right-Ctrl through the next step. Ctrl+Backspace deletes a whole\n\
         WORD in most editors, so if the lift fails you will see far more than\n\
         five characters disappear."
    );
    sleep(Duration::from_secs(3));
    step(
        &mut injector,
        "replace under a held modifier",
        "keep this hello world",
        5,
        "there",
        "keep this hello there — `keep this` untouched",
    )?;

    Ok(())
}

fn step(
    injector: &mut Injector,
    name: &str,
    text: &str,
    take_back: usize,
    replacement: &str,
    expected: &str,
) -> Result<()> {
    println!("\n[{name}] typing {text:?}");
    injector.type_text(text)?;
    sleep(Duration::from_millis(1200));
    println!("  replacing the last {take_back} chars with {replacement:?}");
    injector.replace_last(take_back, replacement)?;
    println!("  expect: {expected}");
    sleep(Duration::from_millis(1500));

    // Clear the field for the next step, and drop the record with it.
    injector.replace_last(usize::MAX, "")?;
    injector.forget_typed();
    sleep(Duration::from_millis(600));
    Ok(())
}
