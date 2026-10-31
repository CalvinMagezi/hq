#!/usr/bin/env bash
# Fails unless an APK is signed by the AgentHQ release key (and only that key) and has the public app id.
#   android-check.sh APK
# Prints "versionCode minSdk" on success. Needs apksigner and aapt2 from the Android SDK build-tools
# (found through ANDROID_HOME or ANDROID_SDK_ROOT, or already on PATH).
set -euo pipefail

# SHA-256 of the release signing certificate. The key is fixed for the life of the app id: a different
# one cannot update installed copies.
EXPECTED_CERT_SHA256="fc6b48a79fdc3dea9045864b7a09a0f9cc992cb24066a3d64391ff412a07295a"
EXPECTED_APP_ID="online.agenthq.app"

[ $# -eq 1 ] && [ -f "$1" ] || { echo "usage: android-check.sh APK" >&2; exit 2; }
apk=$1

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
if [ -n "$sdk" ]; then
    tools=$(find "$sdk/build-tools" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -n 1 || true)
    [ -z "$tools" ] || PATH="$tools:$PATH"
fi
command -v apksigner >/dev/null || { echo "android-check.sh: apksigner not found" >&2; exit 1; }
command -v aapt2 >/dev/null || { echo "android-check.sh: aapt2 not found" >&2; exit 1; }

certs=$(apksigner verify --print-certs "$apk") || { echo "android-check.sh: apksigner rejects $apk" >&2; exit 1; }
# Older apksigner prints "Signer #1 certificate ...", newer ones "V3.0 Signer: certificate ..."; the same key shows once per scheme.
digests=$(printf '%s\n' "$certs" | sed -n 's/^.*[Ss]igner.* certificate SHA-256 digest: //p' | tr 'A-F' 'a-f' | sort -u)
[ "$digests" = "$EXPECTED_CERT_SHA256" ] || {
    echo "android-check.sh: signing certificate is not the release key (got: ${digests:-none})" >&2
    exit 1
}

badging=$(aapt2 dump badging "$apk")
package_line=$(printf '%s\n' "$badging" | sed -n "s/^package: //p" | head -n 1)
[[ "$package_line" == *"name='$EXPECTED_APP_ID'"* ]] || { echo "android-check.sh: app id is not $EXPECTED_APP_ID" >&2; exit 1; }
code=$(printf '%s\n' "$package_line" | sed -n "s/.*versionCode='\([0-9]*\)'.*/\1/p")
# Newer aapt2 calls it minSdkVersion, older ones sdkVersion.
min=$(printf '%s\n' "$badging" | sed -n "s/^\(min\)\{0,1\}[sS]dkVersion:'\([0-9]*\)'.*/\2/p" | head -n 1)
[ -n "$code" ] && [ -n "$min" ] || { echo "android-check.sh: could not read versionCode and minSdk" >&2; exit 1; }
echo "$code $min"
