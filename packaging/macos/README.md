# macOS packaging

Builds `WhisprCatch.app` and a `.dmg` for Apple Silicon.

```sh
packaging/macos/build-dmg.sh
# → dist/WhisprCatch.app
# → dist/WhisprCatch-<version>-arm64.dmg
```

The ONNX Runtime is **statically linked** into the binary, so the bundle is just
the single executable + icon + `Info.plist`. No dylibs to ship.

## Permissions

WhisprCatch needs three macOS privacy grants, all requested on first run:

| Permission | Why | Pane |
|---|---|---|
| **Accessibility** | type transcribed text into the focused app | Privacy › Accessibility |
| **Input Monitoring** | see the push-to-talk key globally | Privacy › Input Monitoring |
| **Microphone** | capture speech while the key is held | Privacy › Microphone |

The first-run wizard opens these panes and shows the system Accessibility prompt.
It never blocks on them: macOS caches a TCC grant **per process**, so a
permission the user just granted still reads as denied until the app restarts.
Gating setup on that check traps the user in the wizard forever — hence the
"Continue — I'll grant these later" escape and the Settings › Permissions card.
`whisper-catch doctor` prints the live status of each grant.

## Signing

Releases are signed with a **self-signed certificate**, generated once:

```sh
packaging/macos/make-signing-cert.sh          # → ~/.whisprcatch/signing/*.p12
SIGN_P12=~/.whisprcatch/signing/whisprcatch-signing.p12 \
SIGN_P12_PASSWORD=… packaging/macos/build-dmg.sh
```

### Why not ad-hoc

macOS keys a permission grant to the app's *designated requirement*. Ad-hoc
signing produces:

```
designated => cdhash H"d2f74084…"
```

which changes on **every build** — so each update looks like a different app and
all three grants are silently dropped. Signing with a certificate produces:

```
designated => identifier "com.whisprcatch.app" and certificate root = H"6191…"
```

which is stable for as long as the same cert is reused. The certificate does not
need to be trusted, or issued by Apple, for this to hold. `codesign` will not
select an untrusted cert by name (`CSSMERR_TP_NOT_TRUSTED`), so `build-dmg.sh`
selects it by SHA-1, which works regardless.

> **Keep the `.p12` and its password.** Regenerating the certificate forces every
> existing user to re-grant all three permissions by hand.

CI reads it from the `MACOS_CERT_P12` (base64) and `MACOS_CERT_PASSWORD` secrets
and fails the release if they are missing, rather than quietly shipping an ad-hoc
build that would reset everyone's permissions.

### Gatekeeper

A self-signed build is not notarized, so Gatekeeper blocks it — and since
**macOS 15 there is no right-click → Open bypass**; the user would have to dig
through System Settings › Privacy & Security › "Open Anyway".

The Homebrew cask sidesteps this instead: Gatekeeper's check is triggered by the
quarantine flag on downloads, so after Homebrew has verified the dmg against its
published SHA-256, the cask's `postflight` clears that flag and the app opens
normally. Installing via `brew` is therefore the supported path — see
[`packaging/homebrew/whisprcatch.rb`](../homebrew/whisprcatch.rb).

### If you later buy a Developer ID ($99/yr)

Set `SIGN_ID` instead of `SIGN_P12`, then notarize and staple:

```sh
export SIGN_ID="Developer ID Application: Your Name (TEAMID)" NOTARIZE=1
packaging/macos/build-dmg.sh
xcrun notarytool submit dist/WhisprCatch-*-arm64.dmg \
  --apple-id "you@example.com" --team-id TEAMID --password "app-specific-pw" --wait
xcrun stapler staple dist/WhisprCatch-*-arm64.dmg
```

That removes the quarantine workaround and lets a plain dmg download work. Note
it also **changes the designated requirement**, so the switchover resets
permissions once for existing users.
