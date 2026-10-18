#!/usr/bin/env bash
# Builds the spike with different network security configs and loads the UI on 127.0.0.1 and localhost, recording what the WebView does.
set -uo pipefail
SP=$1; A=$SP/e3/apk; export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
adb_() { "$HOME/Library/Android/sdk/platform-tools/adb" -s emulator-5572 "$@"; }
run_variant() { # name, config-domains (space separated or NONE), hosts to try
  name=$1; domains=$2; hosts=$3
  rm -rf "$SP/e3/var"; cp -R "$A" "$SP/e3/var"
  if [ "$domains" = NONE ]; then
    sed -i '' 's# android:networkSecurityConfig="@xml/network_security_config"##' "$SP/e3/var/AndroidManifest.xml"; rm -f "$SP/e3/var/res/xml/network_security_config.xml"
  else
    { echo '<?xml version="1.0" encoding="utf-8"?><network-security-config><domain-config cleartextTrafficPermitted="true">'; for d in $domains; do echo "<domain includeSubdomains=\"false\">$d</domain>"; done; echo '</domain-config></network-security-config>'; } > "$SP/e3/var/res/xml/network_security_config.xml"
  fi
  DEBUGGABLE=1 "$SP/e3/var/build.sh" "$SP/e3/bin/hq-android-arm64" "$SP/e3/web" "$SP/e3/var.apk" >/dev/null 2>&1 || { echo "$name: build failed"; return; }
  adb_ install -r -g "$SP/e3/var.apk" >/dev/null 2>&1; adb_ shell pm grant example.hq.spike android.permission.POST_NOTIFICATIONS >/dev/null 2>&1
  for h in $hosts; do
    adb_ shell am force-stop example.hq.spike; sleep 2; adb_ logcat -c
    adb_ shell am start -n example.hq.spike/.MainActivity --es host "$h" --ez direct true >/dev/null 2>&1; sleep 14
    echo "== $name, UI on $h"
    adb_ logcat -d -s HQSPIKE:V HQSPIKE-JS:V 2>&1 | grep -E "webview error|PROBE|page finished" | sed -E 's/^[0-9-]+ [0-9:.]+ +[0-9]+ +[0-9]+ [A-Z] //' | sed -E 's/"ua":"[^"]*",?//' | cut -c1-260 | head -4
  done
}
run_variant "no network security config, direct WebView load" NONE "127.0.0.1 localhost"
run_variant "config lists only 127.0.0.1, direct WebView load" "127.0.0.1" "localhost"
