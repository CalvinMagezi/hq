# Source this with WORK set to a scratch directory that holds the hq copies. Run dirs go under $HOME
# (short path, not under a temp dir: the sandbox refuses a run dir inside a writable temp directory).
: "${WORK:?set WORK to a scratch directory}"
T=$WORK
UIDN=$(id -u)
start_host() { # label binary: run an isolated host as a launchd job with its own label
  label=$1; bin=$2; dir=$HOME/.hqs-spike/$label
  mkdir -p -m 700 "$HOME/.hqs-spike" "$dir"
  cat > "$T/$label.plist" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.hq.spike.$label</string>
<key>ProgramArguments</key><array><string>$bin</string><string>host</string><string>serve</string><string>--dir</string><string>$dir</string></array>
<key>EnvironmentVariables</key><dict><key>HQ_CONFIG_PATH</key><string>$T/config.yaml</string><key>PATH</key><string>/usr/bin:/bin:/usr/sbin:/sbin</string></dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><false/>
<key>StandardOutPath</key><string>$T/$label.log</string><key>StandardErrorPath</key><string>$T/$label.log</string>
</dict></plist>
PL
  launchctl bootstrap "gui/$UIDN" "$T/$label.plist" && sleep 2 && echo "started com.hq.spike.$label"
}
stop_host() { # label
  launchctl bootout "gui/$UIDN/com.hq.spike.$1" 2>&1 | head -1
  rm -rf "$HOME/.hqs-spike/$1" "$T/$1.plist"
}
