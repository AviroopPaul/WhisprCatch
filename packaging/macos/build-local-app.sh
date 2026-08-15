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
# It IS signed with the same certificate as the release, so its permission
# grants survive rebuilds the same way (see make-signing-cert.sh).
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

if [ -f dist/AppIcon.icns ]; then
  cp dist/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

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

if [ -f "$SIGN_P12" ] && [ -n "${SIGN_P12_PASSWORD:-}" ]; then
  echo "==> Signing with the release certificate"
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
  echo "==> Signing ad-hoc (set SIGN_P12_PASSWORD to use the release cert)"
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
