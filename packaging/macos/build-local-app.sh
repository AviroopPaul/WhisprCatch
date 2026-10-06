#!/usr/bin/env bash
#
# Builds "WhisprCatch Local.app" — a side-by-side debug build for testing on
# macOS without disturbing the installed release.
#
#   packaging/macos/build-local-app.sh
#   open -a "WhisprCatch Local"
#
# It differs from the release bundle in three ways that matter:
#
#   * a separate bundle id (com.whisprcatch.app.local), so it gets its OWN
#     Accessibility / Input Monitoring grants and can never collide with the
#     installed app's TCC entries — the two show up as separate rows in
#     System Settings
#   * RUST_LOG baked in via LSEnvironment, so a GUI launch still logs
#   * lives in dist-local/, never packaged into a dmg
#
# It is signed with a certificate, never ad-hoc, so its permission grants
# survive rebuilds (see make-signing-cert.sh): the release certificate when
# SIGN_P12_PASSWORD is set, otherwise a local-only one created on first run in
# ~/.whisprcatch/signing/local-dev.p12. ADHOC=1 forces the old behaviour.
#
# Install it like any app:  ditto "dist-local/WhisprCatch Local.app" "/Applications/WhisprCatch Local.app"
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

APP_NAME="WhisprCatch Local"
BUNDLE_ID="com.whisprcatch.app.local"
BIN="target/release/whisper-catch"
DIST="dist-local"
ENTITLEMENTS="packaging/macos/entitlements.plist"
RUST_LOG_LEVEL="${RUST_LOG_LEVEL:-debug}"
SIGN_P12="${SIGN_P12:-$HOME/.whisprcatch/signing/whisprcatch-signing.p12}"

VERSION="$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)"

echo "==> Building release binary"
cargo build --release -p whisper-catch

echo "==> Assembling $APP_NAME.app"
APP="$DIST/$APP_NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/whisper-catch"
chmod +x "$APP/Contents/MacOS/whisper-catch"

# Render the icon from the PNG in the repo, like build-dmg.sh, rather than
# reusing whatever dist/ holds from an older dmg build (or nothing, on a fresh
# checkout, which left the local app with a generic icon).
echo "==> Rendering AppIcon.icns"
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
gen() { sips -z "$1" "$1" assets/icon-512.png --out "$ICONSET/$2" >/dev/null; }
gen 16 icon_16x16.png;    gen 32 icon_16x16@2x.png
gen 32 icon_32x32.png;    gen 64 icon_32x32@2x.png
gen 128 icon_128x128.png; gen 256 icon_128x128@2x.png
gen 256 icon_256x256.png; gen 512 icon_256x256@2x.png
gen 512 icon_512x512.png; cp assets/icon-512.png "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

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
    <key>LSUIElement</key><true/>
    <key>LSEnvironment</key>
    <dict>
        <key>RUST_LOG</key><string>${RUST_LOG_LEVEL}</string>
        <key>RUST_BACKTRACE</key><string>1</string>
    </dict>
    <key>NSMicrophoneUsageDescription</key>
    <string>WhisprCatch transcribes your speech on-device while you hold the dictation key. Audio never leaves your machine.</string>
</dict>
</plist>
PLIST

# --- sign with the release certificate (same stable-identity machinery) ------
KEYCHAIN=""; KEYCHAIN_DIR=""; LIST_CHANGED=0; ORIG_KEYCHAINS=()
cleanup_keychain() {
  if [ "$LIST_CHANGED" = "1" ]; then
    security list-keychains -d user -s ${ORIG_KEYCHAINS[@]+"${ORIG_KEYCHAINS[@]}"} 2>/dev/null || true
  fi
  [ -n "$KEYCHAIN" ] && security delete-keychain "$KEYCHAIN" 2>/dev/null || true
  [ -n "$KEYCHAIN_DIR" ] && rm -rf "$KEYCHAIN_DIR" || true
}
trap cleanup_keychain EXIT

# Without the release password, sign with a local-only certificate instead of
# ad-hoc. Ad-hoc pins the designated requirement to the cdhash, so every
# rebuild looked like a new app to macOS and silently dropped all three
# permission grants; that is what made local testing feel buggy. The local cert
# is made once, lives beside the release one, and never ships anywhere.
LOCAL_P12="$HOME/.whisprcatch/signing/local-dev.p12"
LOCAL_PW_FILE="$HOME/.whisprcatch/signing/local-dev.password"
if [ -z "${SIGN_P12_PASSWORD:-}" ] && [ -z "${ADHOC:-}" ]; then
  if [ ! -f "$LOCAL_P12" ]; then
    echo "==> Creating a local-only signing certificate (once): $LOCAL_P12"
    mkdir -p "$(dirname "$LOCAL_P12")"; chmod 700 "$(dirname "$LOCAL_P12")"
    T="$(mktemp -d)"
    openssl rand -hex 24 > "$LOCAL_PW_FILE"; chmod 600 "$LOCAL_PW_FILE"
    # /usr/bin/openssl (LibreSSL) writes a .p12 `security import` accepts;
    # OpenSSL 3+ defaults to a format it rejects.
    /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes \
      -keyout "$T/key.pem" -out "$T/cert.pem" -days 3650 \
      -subj "/CN=WhisprCatch Local Dev/O=WhisprCatch" \
      -addext "basicConstraints=critical,CA:false" \
      -addext "keyUsage=critical,digitalSignature" \
      -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null
    /usr/bin/openssl pkcs12 -export -out "$LOCAL_P12" -inkey "$T/key.pem" \
      -in "$T/cert.pem" -passout "pass:$(cat "$LOCAL_PW_FILE")" \
      -name "WhisprCatch Local Dev" 2>/dev/null
    chmod 600 "$LOCAL_P12"
    rm -rf "$T"
  fi
  SIGN_P12="$LOCAL_P12"
  SIGN_P12_PASSWORD="$(cat "$LOCAL_PW_FILE")"
  SIGN_LABEL="the local dev certificate"
fi

if [ -f "$SIGN_P12" ] && [ -n "${SIGN_P12_PASSWORD:-}" ]; then
  echo "==> Signing with ${SIGN_LABEL:-the release certificate}"
  KEYCHAIN_DIR="$(mktemp -d)"
  KEYCHAIN="$KEYCHAIN_DIR/whisprcatch-local.keychain-db"
  KC_PASS="$(openssl rand -hex 24)"
  security create-keychain -p "$KC_PASS" "$KEYCHAIN"
  security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security import "$SIGN_P12" -k "$KEYCHAIN" -P "$SIGN_P12_PASSWORD" -T /usr/bin/codesign -f pkcs12 >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null 2>&1
  while IFS= read -r kc; do
    kc="${kc#"${kc%%[![:space:]]*}"}"; kc="${kc#\"}"; kc="${kc%\"}"
    [ -n "$kc" ] && ORIG_KEYCHAINS+=("$kc")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s ${ORIG_KEYCHAINS[@]+"${ORIG_KEYCHAINS[@]}"} "$KEYCHAIN"
  LIST_CHANGED=1
  SIGN_ID="$(security find-identity -p codesigning "$KEYCHAIN" | grep -o '[0-9A-F]\{40\}' | head -1)"
  codesign --force --options runtime --entitlements "$ENTITLEMENTS" \
    --keychain "$KEYCHAIN" --sign "$SIGN_ID" "$APP/Contents/MacOS/whisper-catch"
  codesign --force --options runtime --entitlements "$ENTITLEMENTS" \
    --keychain "$KEYCHAIN" --sign "$SIGN_ID" "$APP"
else
  echo "==> Signing ad-hoc (ADHOC=1)"
  echo "    note: ad-hoc means macOS will drop this app's permissions on every rebuild"
  codesign --force --entitlements "$ENTITLEMENTS" --sign - "$APP/Contents/MacOS/whisper-catch"
  codesign --force --entitlements "$ENTITLEMENTS" --sign - "$APP"
fi

xattr -cr "$APP" 2>/dev/null || true

echo "==> Designated requirement"
codesign -d -r- "$APP" 2>&1 | grep -v '^Executable=' | sed 's/^/    /'

echo ""
echo "Done: $APP"
echo ""
echo "  Run it with logs:"
echo "    open -a \"$PWD/$APP\" --stderr /tmp/whisprcatch-local.log"
echo "    tail -f /tmp/whisprcatch-local.log"
echo ""
echo "  It needs its OWN Accessibility + Input Monitoring grants"
echo "  (it is a separate bundle id from the installed release)."
exit 0
