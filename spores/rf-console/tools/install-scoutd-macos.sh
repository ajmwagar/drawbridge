#!/bin/sh
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LABEL=com.ajmwagar.rfscoutd
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
PYTHON=${PYTHON:-$(command -v python3)}
mkdir -p "$(dirname "$PLIST")" "$HOME/Library/Logs"

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$PYTHON</string>
    <string>$ROOT/tools/scoutd.py</string>
    <string>--listen</string><string>127.0.0.1:8798</string>
    <string>--page</string><string>$ROOT/web/index.html</string>
    <string>--antenna</string><string>AR-771</string>
  </array>
  <key>WorkingDirectory</key><string>$ROOT</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/rfscoutd.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/rfscoutd.log.err</string>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
echo "installed $PLIST"
echo "status: curl -s http://127.0.0.1:8798/healthz"
