#!/usr/bin/env bash
#
# Creates the self-signed code-signing certificate WhisprCatch releases are
# signed with. Run this ONCE, then keep the .p12 forever.
#
#   packaging/macos/make-signing-cert.sh
#
# Why this exists
# ---------------
# macOS ties a TCC permission grant (Accessibility, Input Monitoring,
# Microphone) to the app's *designated requirement*. For an ad-hoc signature
# that requirement is the cdhash:
#
#     designated => cdhash H"d2f74084..."
#
# which changes on every single build — so every update looks like a brand-new
# app and macOS silently drops the permissions. A push-to-talk app that loses
# its event tap after an update just appears broken.
#
# Signing with a certificate instead pins the requirement to the cert:
#
#     designated => identifier "com.whisprcatch.app" and certificate root = H"6191..."
#
# That is stable across rebuilds, so grants survive updates. The cert does not
# need to be trusted (or from Apple) for this to hold — see build-dmg.sh.
#
# ---------------------------------------------------------------------------
#  KEEP THE .p12 AND ITS PASSWORD. If they are lost, the next release is signed
#  by a different cert, and every existing user has to re-grant all three
#  permissions by hand. Back it up somewhere durable (password manager).
# ---------------------------------------------------------------------------
set -euo pipefail

OUT_DIR="${OUT_DIR:-$HOME/.whisprcatch/signing}"
CN="${CN:-WhisprCatch Self-Signed}"
ORG="${ORG:-WhisprCatch}"
DAYS="${DAYS:-3650}"
P12="$OUT_DIR/whisprcatch-signing.p12"

if [ -e "$P12" ]; then
  echo "refusing to overwrite an existing cert: $P12"
  echo ""
  echo "If you regenerate it, every user's macOS permissions reset on the next"
  echo "update. Delete it by hand only if you are certain that's what you want."
  exit 1
fi

# A password is required — openssl will happily make a passwordless .p12, but
# GitHub Actions secrets and `security import` both behave better with one.
if [ -z "${P12_PASSWORD:-}" ]; then
  # Not `tr </dev/urandom | head -c N` — head exits early, tr takes SIGPIPE, and
  # `set -o pipefail` then aborts the script.
  P12_PASSWORD="$(openssl rand -hex 24)"
  GENERATED_PW=1
fi

mkdir -p "$OUT_DIR"
chmod 700 "$OUT_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Generating a $DAYS-day self-signed code-signing certificate"
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -days "$DAYS" \
  -subj "/CN=$CN/O=$ORG" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null

echo "==> Bundling into a .p12"
openssl pkcs12 -export \
  -out "$P12" -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -passout "pass:$P12_PASSWORD" -name "$CN" 2>/dev/null
chmod 600 "$P12"

# The SHA-1 of the DER cert is what ends up in the designated requirement, and
# what every user's TCC database will key off. Worth recording.
FINGERPRINT="$(openssl x509 -in "$TMP/cert.pem" -noout -fingerprint -sha1 \
                | sed 's/.*=//; s/://g')"

cat > "$OUT_DIR/README.txt" <<TXT
WhisprCatch code-signing certificate
====================================
Created : $(date -u '+%Y-%m-%dT%H:%M:%SZ')
Subject : CN=$CN, O=$ORG
Expires : $DAYS days from creation
SHA-1   : $FINGERPRINT

This certificate is the stable identity behind every WhisprCatch macOS build.
macOS keys the user's Accessibility / Input Monitoring / Microphone grants to
it. Replacing it forces every user to re-grant those permissions.

Back up whisprcatch-signing.p12 AND its password.
TXT

echo ""
echo "Done."
echo "  cert:        $P12"
echo "  SHA-1:       $FINGERPRINT"
if [ "${GENERATED_PW:-0}" = "1" ]; then
  echo "  password:    $P12_PASSWORD"
  echo ""
  echo "  ^ generated for you — SAVE IT NOW, it is not stored anywhere."
fi
echo ""
echo "Next:"
echo "  1. Back up the .p12 + password (password manager, not this repo)."
echo "  2. Build a signed release locally:"
echo "       SIGN_P12=\"$P12\" SIGN_P12_PASSWORD=… packaging/macos/build-dmg.sh"
echo "  3. Wire up CI:"
echo "       base64 -i \"$P12\" | pbcopy"
echo "       gh secret set MACOS_CERT_P12       # paste"
echo "       gh secret set MACOS_CERT_PASSWORD  # the password above"
