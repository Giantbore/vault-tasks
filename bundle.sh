#!/bin/zsh
# Builds a release binary and wraps it in "Vault Tasks.app".
# Usage: ./bundle.sh            -> builds into ./dist
#        ./bundle.sh --install  -> also copies to ~/Applications
set -e
cd "$(dirname "$0")"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
cargo build --release

APP="dist/Vault Tasks.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/vault-tasks "$APP/Contents/MacOS/vault-tasks"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Vault Tasks</string>
  <key>CFBundleDisplayName</key><string>Vault Tasks</string>
  <key>CFBundleIdentifier</key><string>com.local.vault-tasks</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleExecutable</key><string>vault-tasks</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"
echo "Built $APP"

if [[ "$1" == "--install" ]]; then
  mkdir -p ~/Applications
  rm -rf ~/Applications/"Vault Tasks.app"
  cp -R "$APP" ~/Applications/
  echo "Installed to ~/Applications/Vault Tasks.app"
fi
