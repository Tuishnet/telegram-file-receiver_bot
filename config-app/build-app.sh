#!/bin/bash
# Собирает «Настройки бота.app» в этой папке
set -euo pipefail
cd "$(dirname "$0")"

swift build -c release
APP="FileReceiverConfig.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp icon/AppIcon.icns "$APP/Contents/Resources/"
cp .build/release/FileReceiverConfig "$APP/Contents/MacOS/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>FileReceiverConfig</string>
    <key>CFBundleIdentifier</key><string>local.file-receiver.config</string>
    <key>CFBundleName</key><string>Настройки бота</string>
    <key>CFBundleDisplayName</key><string>Настройки бота</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP" >/dev/null 2>&1 || true
# Пользовательская иконка — Finder и Dock показывают её сразу, без ожидания кэша
swift icon/set-icon.swift icon/icon.png "$(pwd)/$APP" >/dev/null 2>&1 || true
echo "Готово: $(pwd)/$APP"
