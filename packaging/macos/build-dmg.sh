#!/usr/bin/env bash
#
# Builds WhisprCatch.app and a distributable .dmg for macOS (Apple Silicon).
#
#   packaging/macos/build-dmg.sh
#
# Output: dist/WhisprCatch.app and dist/WhisprCatch-<version>-arm64.dmg
#
# Signing — three modes, best first:
#
#   SIGN_P12=… SIGN_P12_PASSWORD=…   self-signed cert (packaging/macos/make-signing-cert.sh)
#   SIGN_ID="Developer ID Application: … (TEAMID)"
#   neither                          ad-hoc — DEVELOPMENT ONLY
#
# Why ad-hoc is not good enough for a release: macOS keys the user's
# Accessibility / Input Monitoring / Microphone grants to the app's designated
# requirement, and for an ad-hoc signature that requirement is the cdhash —
# which changes on every build. Every update then looks like a different app and
# the permissions are silently dropped. Signing with a cert (even an untrusted
# self-signed one) pins the requirement to the certificate instead, so grants
# survive updates. See make-signing-cert.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

APP_NAME="WhisprCatch"
BUNDLE_ID="com.whisprcatch.app"
BIN="target/release/whisper-catch"
DIST="dist"
SIGN_P12="${SIGN_P12:-}"
SIGN_ID="${SIGN_ID:--}"                 # "-" = ad-hoc
ENTITLEMENTS="packaging/macos/entitlements.plist"

VERSION="$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)"
[ -n "$VERSION" ] || { echo "could not read version from Cargo.toml"; exit 1; }

echo "==> WhisprCatch $VERSION — building release binary"
if [ ! -x "$BIN" ]; then
  cargo build --release -p whisper-catch
fi
ARCH="$(uname -m)"   # arm64 on Apple Silicon

echo "==> Rendering AppIcon.icns"
rm -rf "$DIST"
ICONSET="$DIST/AppIcon.iconset"
mkdir -p "$ICONSET"
SRC_ICON="assets/icon-512.png"
gen() { sips -z "$1" "$1" "$SRC_ICON" --out "$ICONSET/$2" >/dev/null; }
gen 16   icon_16x16.png
gen 32   icon_16x16@2x.png
gen 32   icon_32x32.png
gen 64   icon_32x32@2x.png
gen 128  icon_128x128.png
gen 256  icon_128x128@2x.png
gen 256  icon_256x256.png
gen 512  icon_256x256@2x.png
gen 512  icon_512x512.png
gen 1024 icon_512x512@2x.png
iconutil -c icns "$ICONSET" -o "$DIST/AppIcon.icns"

echo "==> Assembling $APP_NAME.app"
APP="$DIST/$APP_NAME.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/whisper-catch"
chmod +x "$APP/Contents/MacOS/whisper-catch"
cp "$DIST/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundleExecutable</key><string>whisper-catch</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <!-- Menu-bar app: no Dock icon -->
    <key>LSUIElement</key><true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>WhisprCatch transcribes your speech on-device while you hold the dictation key. Audio never leaves your machine.</string>
</dict>
</plist>
PLIST

# --- signing identity ------------------------------------------------------
# When SIGN_P12 is given, the cert is imported into a throwaway keychain that is
# torn down on exit. codesign only searches keychains on the *user search list*
# (--keychain alone is not enough), so the list is edited and restored here.
KEYCHAIN=""
KEYCHAIN_DIR=""
LIST_CHANGED=0
ORIG_KEYCHAINS=()

cleanup_keychain() {
  if [ "$LIST_CHANGED" = "1" ]; then
    security list-keychains -d user -s ${ORIG_KEYCHAINS[@]+"${ORIG_KEYCHAINS[@]}"} 2>/dev/null || true
  fi
  [ -n "$KEYCHAIN" ] && security delete-keychain "$KEYCHAIN" 2>/dev/null || true
  [ -n "$KEYCHAIN_DIR" ] && rm -rf "$KEYCHAIN_DIR" || true
}
trap cleanup_keychain EXIT

if [ -n "$SIGN_P12" ]; then
  [ -f "$SIGN_P12" ] || { echo "SIGN_P12 not found: $SIGN_P12"; exit 1; }
  : "${SIGN_P12_PASSWORD:?SIGN_P12_PASSWORD must be set alongside SIGN_P12}"

  echo "==> Importing signing certificate into a temporary keychain"
  KEYCHAIN_DIR="$(mktemp -d)"
  KEYCHAIN="$KEYCHAIN_DIR/whisprcatch-build.keychain-db"
  # Not `tr </dev/urandom | head -c N`: head exits early, tr takes SIGPIPE, and
  # under `set -o pipefail` that aborts the build.
  KC_PASS="$(openssl rand -hex 24)"
  security create-keychain -p "$KC_PASS" "$KEYCHAIN"
  security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security import "$SIGN_P12" -k "$KEYCHAIN" -P "$SIGN_P12_PASSWORD" \
      -T /usr/bin/codesign -f pkcs12 >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: \
      -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null 2>&1

  while IFS= read -r kc; do
    kc="${kc#"${kc%%[![:space:]]*}"}"          # strip leading whitespace
    kc="${kc#\"}"; kc="${kc%\"}"               # strip surrounding quotes
    [ -n "$kc" ] && ORIG_KEYCHAINS+=("$kc")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s ${ORIG_KEYCHAINS[@]+"${ORIG_KEYCHAINS[@]}"} "$KEYCHAIN"
  LIST_CHANGED=1

  # A self-signed cert is reported CSSMERR_TP_NOT_TRUSTED, so it never shows up
  # in `find-identity -v` and cannot be selected by name. Selecting it by SHA-1
  # works regardless — and trust is irrelevant to what we actually want here,
  # which is a stable designated requirement.
  SIGN_ID="$(security find-identity -p codesigning "$KEYCHAIN" \
              | grep -o '[0-9A-F]\{40\}' | head -1)"
  [ -n "$SIGN_ID" ] || { echo "no code-signing identity inside $SIGN_P12"; exit 1; }
  echo "    identity $SIGN_ID"
fi

if [ "$SIGN_ID" = "-" ]; then
  echo "==> Signing (ad-hoc — development only)"
  CS_ARGS=(--force --entitlements "$ENTITLEMENTS")
else
  echo "==> Signing ($SIGN_ID)"
  CS_ARGS=(--force --options runtime --entitlements "$ENTITLEMENTS")
  # A trusted timestamp only matters for notarization; asking Apple's TSA for
  # one on every build just adds a network call that can fail the release.
  [ -n "${NOTARIZE:-}" ] && CS_ARGS+=(--timestamp)
fi
KC_ARG=()
[ -n "$KEYCHAIN" ] && KC_ARG=(--keychain "$KEYCHAIN")

codesign "${CS_ARGS[@]}" ${KC_ARG[@]+"${KC_ARG[@]}"} --sign "$SIGN_ID" "$APP/Contents/MacOS/whisper-catch"
codesign "${CS_ARGS[@]}" ${KC_ARG[@]+"${KC_ARG[@]}"} --sign "$SIGN_ID" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP" || true

# This line is the whole point of certificate signing — it is what macOS stores
# alongside a permission grant. It must stay identical between releases.
echo "==> Designated requirement (macOS keys TCC grants to this)"
codesign -d -r- "$APP" 2>&1 | grep -v '^Executable=' | sed 's/^/    /'
if [ "$SIGN_ID" = "-" ]; then
  echo "    ^ cdhash-based: changes every build, so permissions reset on update."
fi

echo "==> Building .dmg"
STAGE="$DIST/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
DMG="$DIST/${APP_NAME}-${VERSION}-${ARCH}.dmg"
rm -f "$DMG"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE" "$ICONSET"

if [ "$SIGN_ID" != "-" ]; then
  codesign "${CS_ARGS[@]}" ${KC_ARG[@]+"${KC_ARG[@]}"} --sign "$SIGN_ID" "$DMG"
fi

echo ""
echo "Done:"
echo "  app: $APP"
echo "  dmg: $DMG"
if [ "$SIGN_ID" = "-" ]; then
  echo ""
  echo "  NOTE: ad-hoc signed — development build."
  echo "        Permissions reset on every rebuild, and Gatekeeper blocks it."
  echo "        Use SIGN_P12 for anything users will install."
fi
# Nothing after this — a bare `[ … ] && echo` as the final statement makes the
# script exit non-zero whenever the test is false.
exit 0
