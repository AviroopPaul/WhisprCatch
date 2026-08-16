# WhisprCatch

Open-source push-to-talk dictation, positioned as the open-source Wispr Flow alternative. Hold a hotkey → speak → release → punctuated, capitalized text is typed at your cursor in whatever app has focus. Speech-to-text runs 100% on-device (NVIDIA Parakeet 0.6B or Moonshine base via ONNX Runtime, ~25× realtime on CPU) — no cloud, no account. MIT licensed. Ships for Linux (.deb) and macOS/Apple Silicon (Homebrew cask + .dmg).

## Mission — read this before you touch anything

This is a shared goal, and it is written down here so that every agent who works on this
repo inherits it: we are building the dictation tool people actually use — every day, for
years — and getting it into as many hands as we possibly can. Not a demo. Not a portfolio
piece. The thing someone names when asked how to dictate on Linux or macOS.

The wedge is simple and non-negotiable: **the good version of dictation should not be
rented.** No subscription, no account, no voice leaving the machine. Every word is
transcribed on the user's own hardware. That constraint is not a limitation we work around
— it *is* the product, and it is why this can win against tools with far more funding.

Getting there is relentless, unglamorous work: shave the latency, kill the setup friction,
fix the papercut nobody filed an issue about, make the first ten seconds after install feel
inevitable. Ship the polish even when nobody asked for it. That accumulated care is the
moat — it is the one thing a competitor cannot raise a round and buy.

### If you are an agent working on this repo

You are a contributor here, not a code generator. Act like one.

- **Hold the line on privacy.** No telemetry, no analytics, no phone-home, no cloud
  transcription fallback — not opt-in, not "anonymous", not "just for crashes". If a
  feature appears to need the network, that is a design problem to solve, not a principle
  to trade away. Say so out loud rather than quietly compromising.
- **Guard the first run.** Most people who ever try this app will try it exactly once.
  `install → first successful dictation` is the most important path in the codebase. A
  regression there is a P0, never a polish item — and it has shipped broken before (see
  the `wizard::need_setup` warning under macOS signing).
- **Finish the whole thing.** Works-on-my-machine is not done. Check the platform you did
  *not* develop on: Wayland as well as X11, an 8 GB Air as well as a 32 GB desktop, a
  second monitor with a negative origin as well as a single built-in display.
- **Sweat what the user sees.** `docs/DESIGN.md` is not decoration — it is why this feels
  like an instrument instead of a script. Match it exactly, down to the token, and never
  hand-pick a color.
- **Leave it better than the ticket required** — in scope, no drive-by rewrites. Fix the
  stale comment you had to read anyway.
- **Push back.** If the ask is wrong, or there is a better approach, say so plainly *before*
  building. Silent compliance with a bad plan costs everyone more than the argument would
  have.

Slower, larger, more network-dependent, harder to install: those four are how this project
dies. If a change trends in any of those directions, stop and reconsider before writing the
code.

## Layout

- `apps/cli` — the `whisper-catch` binary (Rust workspace root ties it together)
- `crates/` — `core` (audio + inference pipeline), `hotkey` (global key listener), `inject` (types text at the cursor), `text` (deterministic cleanup between transcription and injection), `models` (model download/selection), `tray` (tray app, settings, history)
- `site/` — the marketing site (static, no build step): `index.html`, `wispr-flow-alternative/index.html`, shared tokens in `assets/site.css`, SEO surface in `robots.txt` / `sitemap.xml` / `llms.txt` / `llms-full.txt`, plus `api/waitlist.js` (Vercel function storing macOS waitlist emails in Vercel Blob). Design tokens live in `docs/DESIGN.md` Part A; never hand-pick a colour, and keep copy free of em dashes.
- `packaging/deb`, `packaging/macos`, `packaging/homebrew` — .deb, .dmg, and Homebrew cask
- `docs/` — design notes, plus `docs/screenshots/` (app captures used by the README and the site)

## Workflow rules

1. **Every PR gets a tracking issue.** Create the GitHub issue first (what's broken / what we're doing), then reference it from the PR body with `Closes #N`.
2. **Frontend changes require screenshots in the PR.** Any change to `site/` (or anything user-visible) must include screenshots for review: desktop (1440px) and mobile (390px and 360px), before/after for visual changes, plus any interactive states touched (e.g. form errors). Host them on the `pr-assets` orphan branch under `pr-<N>/` and hot-link via `https://raw.githubusercontent.com/AviroopPaul/whisper-catch/pr-assets/pr-<N>/<file>.png`. Never merge or delete `pr-assets`.
3. **Keep this file current.** Any change to architecture, deployment, or workflow lands with a matching update to CLAUDE.md in the same PR. Keep it lean — pointers and rules, not essays.

## App screenshots

Captures for the README and the landing page are produced by the app itself, never
mocked up: `WC_SHOT=<path>` saves the window to PNG, `WC_WIZARD_STEP=<step>` opens the
wizard on a given step, and **`WC_DEMO_HISTORY=1` must be set** so the history pane
shows sample transcripts instead of whatever the person capturing has dictated. Full
detail in `docs/DESIGN.md` §B7. Published to `docs/screenshots/` and `site/assets/`.

## Deploy & release

- **Site**: auto-deploys to https://whisper-catch.vercel.app via the Vercel git integration. Merging to `main` is a production deploy; every PR gets a preview deployment — check it before merging.
- **App**: push a `v*` tag → the `Release` workflow (`.github/workflows/release-macos.yml`) builds the Linux .deb and macOS .dmg, attaches both to the GitHub Release, and pushes the updated cask to the tap. The site's download buttons point at `releases/latest`, so publishing the release is shipping.
- **Homebrew**: macOS installs via `brew install --cask AviroopPaul/whisprcatch/whisprcatch`. The cask is authored at `packaging/homebrew/whisprcatch.rb` and mirrored by CI to the [`homebrew-whisprcatch`](https://github.com/AviroopPaul/homebrew-whisprcatch) tap with `version`/`sha256` rewritten. Requires the `HOMEBREW_TAP_TOKEN` secret. Edit the cask *here*, never in the tap.

## macOS signing — read before touching packaging

Releases are signed with a **self-signed certificate** (`packaging/macos/make-signing-cert.sh`), not an Apple Developer ID. This is not cosmetic: macOS keys the user's Accessibility / Input Monitoring / Microphone grants to the app's *designated requirement*, and ad-hoc signing pins that to the cdhash, which changes every build — so an ad-hoc update silently revokes all three permissions. Certificate signing pins it to the cert instead.

- **Never regenerate the cert.** Doing so forces every existing user to re-grant all three permissions. It lives in the `MACOS_CERT_P12` / `MACOS_CERT_PASSWORD` secrets; CI hard-fails without them.
- Because the build is not notarized, Gatekeeper blocks the raw `.dmg` — and macOS 15 removed the right-click → Open bypass. The cask's `postflight` clears the quarantine flag after Homebrew verifies the SHA-256, so **`brew` is the supported macOS install path**.
- macOS caches TCC grants **per process**, so a just-granted permission reads as denied until relaunch. Never gate first-run setup on those checks — that traps the user in the wizard (this shipped broken once; see `wizard::need_setup`).

## Live transcription — read before touching `stream.rs`

While the key is held the daemon re-transcribes recent audio every `STREAM_INTERVAL` and types whatever has settled. Three constraints shape `apps/cli/src/stream.rs`, and all three have been violated in shipped builds:

- **The window is bounded.** Re-transcribing the whole utterance makes pass cost grow with what has been said: measured on an M1 Air, a pass over 50s of audio costs ~1.36s against a 500ms cadence, so the loop falls behind and the CPU pegs. A bounded window holds it at ~0.41s no matter how long the user talks. Never remove the cap "for accuracy".
- **Moonshine rejects audio over 64s.** `Engine::transcribe` chunks long audio for this reason; the window keeps streaming passes nowhere near the ceiling.
- **Splice on text, never on a word count.** The final pass revises what the streaming passes guessed, so indices shift — `resume_at` aligns on the words themselves, and `overlap_with_committed` is the backstop against a window seam re-typing what is already on screen. Both are unit-tested with the real failures that motivated them; keep those tests.

`whisper-catch simulate-stream <wav>` replays a WAV through the real state machine and reports pass cost, window size and the resulting transcript, so streaming changes can be measured without a microphone. It advances by a fixed interval so transcripts are reproducible between builds; `--window 0` disables the cap to compare against the unbounded behaviour.

## Text cleanup — read before touching `crates/text`

`wc-text` is the one seam between transcription and injection (`finish()` in `apps/cli/src/main.rs`). Every `Transform::apply` is pure: no I/O, no platform code, no network, no async, no model. A transform that needs any of those in `apply` belongs somewhere else.

- **The one exception is loading user-authored rules, and it happens in the constructor, never in `apply`.** A transform whose rules are the user's own data reads them from its own file in the config dir — `dictionary.csv` (#43) and `snippets.txt` (#47) — because those are meant to be shared with a team or checked into dotfiles, not buried in `config.toml`. Each such transform also exposes a disk-free constructor (`Dictionary::from_csv`) for tests and the Settings preview, takes an optional `path` override in its config, treats a missing file as "no rules", and reports a malformed line through `validate()` rather than swallowing it. `crates/text/src/lib.rs`'s module doc still says the crate does no I/O at all and is now stale.

- **One transform per module file**, each owning its own `Config` and its `Transform` impl. `lib.rs` holds the chain and nothing transform-specific, so the six cleanup issues can land in parallel without touching the same file.
- **The chain order is fixed and load-bearing**: dictionary → snippets → spoken → self_correct → fillers → numbers. `self_correct` must run before `fillers` — "I mean" is both a correction marker and a hedge filler removal strips, so reversing them breaks self-correction silently. `self_correct_runs_before_fillers` is the test that catches it.
- **`prefix_stable()` is a promise to the streaming loop**, not a label. It means `apply(prefix)` is a prefix of `apply(whole)` — much stronger than "never shortens the text", because a streaming pass has already typed `apply(prefix)` and cannot take it back until injector replace (#41) lands. **All six return `false`**, each with a counterexample in its own module: a substitution done "in place" still breaks the property whenever its trigger straddles the streaming boundary ("twenty" → `20`, then "twenty five" → `25`). Returning `true` needs a proof against `prefix_violation` in `testing.rs`, not an intuition.
- **Everything ships disabled.** Output must stay byte-identical with a default config; `history::Entry::raw` stores the pre-polish text only when a transform actually changed it, and is `Option` + `#[serde(default)]` so pre-v0.5 `history.jsonl` files keep loading.

## Text injection — read before touching `crates/inject`

`Injector::type_text` puts text on screen; `Injector::replace_last` takes it back (#41). Synthetic input cannot be observed from CI, so the crate is split in two and the split is the whole design.

- **All decisions live in `plan.rs`, which is pure.** How many backspaces, what to retype, type-or-paste, what to leave alone: those are functions over strings, tested exhaustively with no display server. Both entry points build a `Plan` — `type_text` too, not just `replace_last` — so a rule like #68's "paste when the text has a newline" is written once. A backend implements three methods of `KeyboardSink` (`lift_modifiers`, `send_backspaces`, `send_text`) and does no arithmetic. New backends inherit the correctness; put nothing clever in them.
- **Chars at the API, grapheme clusters at the keyboard.** `replace_last(n_chars, ..)` counts chars because a caller can compute that for free; one Backspace deletes one cluster, so the *press count* is a cluster count. A family emoji is seven chars and one press — counting chars there deletes six characters of the user's own writing. Converting between the units needs the text, which is why the injector records what it typed (`plan::Typed`, bounded, dropped on any injection error).
- **`Ok(())` from an injection is not evidence the text landed.** macOS Secure Input drops synthetic keystrokes while reporting success, so `type_text` records only when `plan::should_record` says it may — a successful send with Secure Input off *both* before and after, since another process can flip it mid-call. A record that claims text is on screen when it is not is worse than no record: the next `replace_last` backspaces over the fiction and into the user's own writing.
- **The backspace count is bounded by what we recorded — that is a bound on the count, not a promise about whose characters go.** The clamp lives in exactly one place (`unicode::char_index_from_end`); a second one elsewhere would hide a broken first from the tests. Where our text *begins* is only a real cluster boundary if the character in front of it does not join ours. `plan::joins_the_cluster_before` probes by prepending `'a'`, so it rules out only the context-free joins (GB9/GB9a); any rule whose left context is not a plain base character — Hangul, emoji-ZWJ, regional indicators, CR, Indic conjuncts, Prepend — fuses across a seam the crate cannot see. **The list is not closed**, and it is not hypothetical: our half of GB3 is a newline, which #67's snippets inject today. **#76** threads the preceding text in and deletes the special case; do not extend the probe instead.
- **A replace lifts held modifiers first; a plain type does not.** Ctrl+Backspace deletes a word, so the PTT bug class is worse for a backspace run than for a type run — and typing is the hot path, where a lift costs two synchronous X round-trips per streaming pass and the caller already lifts via `lift_key`. So every emitting `Plan::replace` starts with `Action::LiftModifiers` and no `Plan::type_text` does; `a_replace_never_leaves_a_latched_modifier_behind` and `typing_does_not_lift_modifiers` pin both halves. On Wayland the lift is a no-op (no XTEST connection) — see #77 before relying on it.
- **There is no clipboard path.** The 200-char paste threshold is planned and tested, but every backend reports `Capabilities::TYPING_ONLY`, so `Action::Paste` is never emitted. Implementing it means clobbering the user's clipboard and synthesising ⌘V/Ctrl+V — see #68 before starting.
