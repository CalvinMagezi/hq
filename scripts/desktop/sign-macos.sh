#!/usr/bin/env bash
# Signs a built HQ.app and wraps it in a DMG. Runs only on the downloaded artifact, never next to
# build code, so the certificate and the notary key never meet a dependency's build script.
#
# Usage: sign-macos.sh <path/to/HQ.app> <out.dmg>
#
# With no APPLE_CERTIFICATE the app is signed ad hoc: it opens on the machine that has it, and
# Gatekeeper refuses it when downloaded. With the certificate it gets a Developer ID signature and
# the hardened runtime; with the notary key as well, the app and the DMG are notarized and stapled.
#
#   APPLE_CERTIFICATE           base64 of a Developer ID Application .p12
#   APPLE_CERTIFICATE_PASSWORD  its password
#   APPLE_SIGNING_IDENTITY      optional; the first Developer ID Application identity otherwise
#   APPLE_API_ISSUER            App Store Connect issuer id
#   APPLE_API_KEY               App Store Connect key id
#   APPLE_API_KEY_P8            the key's .p8 contents
set -euo pipefail

app=${1:?usage: sign-macos.sh HQ.app out.dmg}
out=${2:?usage: sign-macos.sh HQ.app out.dmg}
[[ $(uname -s) == Darwin ]] || { echo 'sign-macos.sh runs on macOS only' >&2; exit 1; }
[[ -d $app && $app == *.app ]] || { echo "not an app bundle: $app" >&2; exit 1; }
[[ $out == *.dmg && ! -e $out ]] || { echo "the output must be a new .dmg path: $out" >&2; exit 1; }

readonly VOLUME_NAME=HQ
readonly NOTARY_TIMEOUT=30m
# The keychain stays unlocked for the whole run (seconds), however long notarization waits.
readonly KEYCHAIN_LOCK_SECS=7200

umask 077
work=$(mktemp -d)
keychain="$work/signing.keychain-db"
searched=()
cleanup() {
  if ((${#searched[@]})); then
    security list-keychains -d user -s "${searched[@]}" >/dev/null 2>&1 || true
  fi
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

make_dmg() {
  local stage="$work/dmg"
  mkdir -p "$stage"
  ditto "$app" "$stage/$(basename "$app")"
  ln -s /Applications "$stage/Applications"
  hdiutil create -quiet -volname "$VOLUME_NAME" -srcfolder "$stage" -ov -format UDZO "$out"
}

if [[ -z ${APPLE_CERTIFICATE:-} ]]; then
  echo 'No APPLE_CERTIFICATE: signing ad hoc. Gatekeeper will refuse this app when downloaded.' >&2
  codesign --force --deep --sign - "$app"
  codesign --verify --strict --verbose=2 "$app"
  make_dmg
  echo "Ad hoc signed: $out"
  exit 0
fi
: "${APPLE_CERTIFICATE_PASSWORD:?APPLE_CERTIFICATE is set without APPLE_CERTIFICATE_PASSWORD}"

while IFS= read -r entry; do
  searched+=("$entry")
done < <(security list-keychains -d user | sed 's/^[[:space:]]*"//; s/"[[:space:]]*$//')

printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$work/certificate.p12"
password=$(openssl rand -hex 32)
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut "$KEYCHAIN_LOCK_SECS" "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
rm -f "$work/certificate.p12"
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
# codesign finds an identity and its chain only in keychains on the search list.
security list-keychains -d user -s "$keychain" "${searched[@]}"

identity=${APPLE_SIGNING_IDENTITY:-}
if [[ -z $identity ]]; then
  identity=$(security find-identity -v -p codesigning "$keychain" \
    | awk -F'"' '/Developer ID Application:/ {print $2; exit}')
fi
[[ -n $identity ]] || { echo 'no Developer ID Application identity in the certificate' >&2; exit 1; }

sign() {
  codesign --force --options runtime --timestamp --keychain "$keychain" --sign "$identity" "$@"
}

# Inner programs first, then the bundle that seals them; --deep is not used for real signatures.
shopt -s nullglob
main_exe=$(plutil -extract CFBundleExecutable raw "$app/Contents/Info.plist")
for exe in "$app"/Contents/MacOS/*; do
  [[ $(basename "$exe") == "$main_exe" ]] || sign "$exe"
done
sign "$app"
codesign --verify --strict --verbose=2 "$app"

# Uploads one file and waits for Apple's verdict, printing the notary log when it is refused.
notarize() {
  local upload=$1 log id status=0
  log="$work/notary-$(basename "$upload").json"
  id=$(xcrun notarytool submit "$upload" "${notary_auth[@]}" --output-format json --no-wait \
    | /usr/bin/python3 -I -c 'import json, sys; print(json.load(sys.stdin)["id"])')
  xcrun notarytool wait "$id" "${notary_auth[@]}" --timeout "$NOTARY_TIMEOUT" || status=$?
  if ((status != 0)); then
    xcrun notarytool log "$id" "${notary_auth[@]}" "$log" && cat "$log" >&2
    echo "notarization of $(basename "$upload") failed" >&2
    exit 1
  fi
}

staple() {
  xcrun stapler staple "$1"
  xcrun stapler validate "$1"
}

notary_auth=()
if [[ -n ${APPLE_API_KEY_P8:-} && -n ${APPLE_API_KEY:-} && -n ${APPLE_API_ISSUER:-} ]]; then
  printf '%s' "$APPLE_API_KEY_P8" > "$work/AuthKey.p8"
  notary_auth=(--key "$work/AuthKey.p8" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
  ditto -c -k --keepParent "$app" "$work/app.zip"
  notarize "$work/app.zip"
  # A zip cannot hold a ticket; it goes on the app, which the DMG then carries.
  staple "$app"
fi

make_dmg
codesign --force --timestamp --keychain "$keychain" --sign "$identity" "$out"
if ((${#notary_auth[@]})); then
  notarize "$out"
  staple "$out"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$out"
  spctl --assess --type execute --verbose=2 "$app"
  echo "Developer ID signed, notarized and stapled: $out"
else
  echo 'No notary key (APPLE_API_KEY_P8, APPLE_API_KEY, APPLE_API_ISSUER): signed, not notarized.' >&2
  echo "Developer ID signed: $out"
fi
