# WhisprCatch

Open-source push-to-talk dictation, positioned as the open-source Wispr Flow alternative. Hold a hotkey → speak → release → punctuated, capitalized text is typed at your cursor in whatever app has focus. Speech-to-text runs 100% on-device (NVIDIA Parakeet 0.6B or Moonshine base via ONNX Runtime, ~25× realtime on CPU) — no cloud, no account. MIT licensed. Ships for Linux (.deb) and macOS/Apple Silicon (Homebrew cask + .dmg).

## Layout

- `apps/cli` — the `whisper-catch` binary (Rust workspace root ties it together)
- `crates/` — `core` (audio + inference pipeline), `hotkey` (global key listener), `inject` (types text at the cursor), `models` (model download/selection), `tray` (tray app, settings, history)
- `site/` — landing page: a single self-contained `index.html` (no build step) plus `api/waitlist.js` (Vercel function storing macOS waitlist emails in Vercel Blob)
- `packaging/deb`, `packaging/macos`, `packaging/homebrew` — .deb, .dmg, and Homebrew cask
- `docs/` — design notes

## Workflow rules

1. **Every PR gets a tracking issue.** Create the GitHub issue first (what's broken / what we're doing), then reference it from the PR body with `Closes #N`.
2. **Frontend changes require screenshots in the PR.** Any change to `site/` (or anything user-visible) must include screenshots for review: desktop (1440px) and mobile (390px and 360px), before/after for visual changes, plus any interactive states touched (e.g. form errors). Host them on the `pr-assets` orphan branch under `pr-<N>/` and hot-link via `https://raw.githubusercontent.com/AviroopPaul/whisper-catch/pr-assets/pr-<N>/<file>.png`. Never merge or delete `pr-assets`.
3. **Keep this file current.** Any change to architecture, deployment, or workflow lands with a matching update to CLAUDE.md in the same PR. Keep it lean — pointers and rules, not essays.

## Deploy & release

- **Site**: auto-deploys to https://whisper-catch.vercel.app via the Vercel git integration. Merging to `main` is a production deploy; every PR gets a preview deployment — check it before merging.
- **App**: push a `v*` tag → the `Release` workflow (`.github/workflows/release-macos.yml`) builds the Linux .deb and macOS .dmg, attaches both to the GitHub Release, and pushes the updated cask to the tap. The site's download buttons point at `releases/latest`, so publishing the release is shipping.
- **Homebrew**: macOS installs via `brew install --cask AviroopPaul/whisprcatch/whisprcatch`. The cask is authored at `packaging/homebrew/whisprcatch.rb` and mirrored by CI to the [`homebrew-whisprcatch`](https://github.com/AviroopPaul/homebrew-whisprcatch) tap with `version`/`sha256` rewritten. Requires the `HOMEBREW_TAP_TOKEN` secret. Edit the cask *here*, never in the tap.

## macOS signing — read before touching packaging

Releases are signed with a **self-signed certificate** (`packaging/macos/make-signing-cert.sh`), not an Apple Developer ID. This is not cosmetic: macOS keys the user's Accessibility / Input Monitoring / Microphone grants to the app's *designated requirement*, and ad-hoc signing pins that to the cdhash, which changes every build — so an ad-hoc update silently revokes all three permissions. Certificate signing pins it to the cert instead.

- **Never regenerate the cert.** Doing so forces every existing user to re-grant all three permissions. It lives in the `MACOS_CERT_P12` / `MACOS_CERT_PASSWORD` secrets; CI hard-fails without them.
- Because the build is not notarized, Gatekeeper blocks the raw `.dmg` — and macOS 15 removed the right-click → Open bypass. The cask's `postflight` clears the quarantine flag after Homebrew verifies the SHA-256, so **`brew` is the supported macOS install path**.
- macOS caches TCC grants **per process**, so a just-granted permission reads as denied until relaunch. Never gate first-run setup on those checks — that traps the user in the wizard (this shipped broken once; see `wizard::need_setup`).
