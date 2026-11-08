#!/bin/zsh
# Builds "Vault Tasks.app".
# Usage: ./bundle.sh [--install] [--universal] [--zip]
#   (no flags)   build for this Mac into target/bundle.noindex
#   --install    also install/update ~/Applications/Vault Tasks.app
#   --universal  build one app that runs on Apple Silicon and Intel Macs
#   --zip        also write target/Vault-Tasks-<version>-macOS.zip for sharing
# The ".noindex" folder keeps Spotlight from listing build copies as extra apps.
set -e
cd "$(dirname "$0")"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

INSTALL=0; UNIVERSAL=0; ZIP=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    --universal) UNIVERSAL=1 ;;
    --zip) ZIP=1 ;;
    *) echo "Unknown option: $arg"; exit 1 ;;
  esac
done

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

if [[ $UNIVERSAL == 1 ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/vault-tasks \
    target/aarch64-apple-darwin/release/vault-tasks \
    target/x86_64-apple-darwin/release/vault-tasks
  BIN=target/universal/vault-tasks
else
  cargo build --release
  BIN=target/release/vault-tasks
fi

APP="target/bundle.noindex/Vault Tasks.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/vault-tasks"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Vault Tasks</string>
  <key>CFBundleDisplayName</key><string>Vault Tasks</string>
  <key>CFBundleIdentifier</key><string>com.local.vault-tasks</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>vault-tasks</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"
echo "Built $APP ($VERSION)"

if [[ $ZIP == 1 ]]; then
  OUT="target/Vault-Tasks-$VERSION-macOS.zip"
  rm -f "$OUT"
  # ditto keeps the app's metadata and signature intact.
  ditto -c -k --keepParent "$APP" "$OUT"
  echo "Zipped $OUT"
fi

if [[ $INSTALL == 1 ]]; then
  DEST=~/Applications/"Vault Tasks.app"
  mkdir -p "$DEST"
  # Update in place so macOS keeps treating it as the same app.
  rsync -a --delete "$APP/" "$DEST/"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$DEST"
  touch "$DEST"  # nudge Finder/Dock to reload the icon
  echo "Installed to $DEST"
fi
