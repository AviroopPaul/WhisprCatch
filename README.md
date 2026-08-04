<div align="center">

<img src="assets/icon-512.png" width="96" alt="WhisprCatch icon">

# WhisprCatch

**Hold a key. Speak. Punctuated text appears wherever your cursor is.**

Local push-to-talk dictation for **Linux** — no cloud, no account, no audio leaving your machine.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/AviroopPaul/whisper-catch)](https://github.com/AviroopPaul/whisper-catch/releases/latest)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-lightgrey.svg)](#install)

[**whisper-catch.vercel.app**](https://whisper-catch.vercel.app)

</div>

---

![WhisprCatch demo — the push-to-talk key is held, a listening pill appears, and the spoken sentence is typed punctuated into a Slack message](docs/demo.gif)

## Why

- **On-device and private.** All inference runs locally via ONNX Runtime. Audio is never written to disk and never leaves the machine.
- **Real punctuation and capitalization.** The model emits properly punctuated text — no "period" or "comma" voice commands.
- **Fast.** ~25× realtime on CPU; live typing streams words while you're still speaking, the rest lands the moment you release the key.

## Install

1. Download the `.deb` from the [latest release](https://github.com/AviroopPaul/whisper-catch/releases/latest).
2. Double-click it (or right-click → *Open with App Center*) and install.
3. Launch **WhisprCatch** from your app menu.

Or from the terminal:

```sh
sudo apt install ./whisper-catch_amd64.deb
whisper-catch ptt
```

A first-run wizard handles keyboard permission (a one-time polkit prompt) and the model download (NVIDIA Parakeet 0.6B, ~660 MB, resumable, SHA-256 verified). Ubuntu/Debian, x86-64.

## Usage

Hold **Right Alt**, speak, release — the transcription is typed into whatever window has focus. A tray icon shows recording state, session stats, and shortcuts; `whisper-catch settings` opens settings and your local transcription history.

Configuration lives at `~/.config/whisper-catch/config.toml`:

| Key | Default | Description |
| --- | --- | --- |
| `key` | `ralt` | Push-to-talk key (`ralt`, `lalt`, `rctrl`, `lctrl`, `super`, `f13`, `scrolllock`, …) |
| `model` | `parakeet` | `parakeet` (best accuracy) or `moonshine` (tiny, ~64 MB) |
| `streaming` | `true` | Type words live while speaking instead of all at once on release |
| `overlay` | `true` | Show the floating recording pill while dictating |
| `history` | `true` | Keep a local log of transcriptions (`history.jsonl`) |
| `layout` | auto | XKB layout to type against on Wayland (`gb`, `us+dvorak`). Detected from the session when unset — set it if detection guesses wrong |

## How it works

Speech models run as int8 ONNX on the CPU via ONNX Runtime ([transcribe-rs](https://crates.io/crates/transcribe-rs)) — no GPU, no network. The hotkey is a raw evdev listener (works on X11 and Wayland), the mic is kept warm with a 300 ms pre-roll so the first syllable isn't clipped, and text is injected into the focused window — XTEST on X11, and on Wayland an in-process uinput virtual keyboard typed against your XKB layout, since XTEST never reaches Wayland-native windows.

Workspace: `crates/core` (capture, resample, engine), `crates/hotkey`, `crates/inject`, `crates/models`, `crates/tray`, `apps/cli`. See [`SCOPE.md`](SCOPE.md) for the full design doc.

## Building from source

```sh
sudo apt install cmake clang libasound2-dev
cargo build --release -p whisper-catch
```

For a `.deb`: `cargo install cargo-deb`, then `cd apps/cli && cargo deb`.

## Roadmap

- **macOS port** — in progress; [join the waitlist](https://whisper-catch.vercel.app) for one email when it ships.
- **Wayland text injection cascade** — wlroots virtual-keyboard → uinput fallback.
- **Streaming-native model** — true incremental decoding instead of rolling re-transcription.

## License

[MIT](LICENSE) © Aviroop Paul
