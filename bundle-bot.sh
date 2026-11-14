#!/bin/bash
# Упаковывает собранного бота в FileReceiverBot.app (нужно для уведомлений macOS).
# Обычно это делает приложение «Настройки бота» само перед запуском.
set -euo pipefail
cd "$(dirname "$0")"

APP="FileReceiverBot.app"
BIN="target/release/file_receiver"
[ -x "$BIN" ] || cargo build --release

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
rm -f "$APP/Contents/MacOS/file_receiver"
cp "$BIN" "$APP/Contents/MacOS/file_receiver"
[ -f config-app/icon/AppIcon.icns ] && cp config-app/icon/AppIcon.icns "$APP/Contents/Resources/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>file_receiver</string>
    <key>CFBundleIdentifier</key><string>local.file-receiver.bot</string>
    <key>CFBundleName</key><string>File Receiver</string>
    <key>CFBundleDisplayName</key><string>File Receiver</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>2.0</string>
    <key>CFBundleVersion</key><string>2</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP" >/dev/null 2>&1 || true
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" || true
echo "Готово: $(pwd)/$APP"
