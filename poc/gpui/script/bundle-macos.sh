#!/bin/sh
# Builds the prototype as a macOS app bundle, so that the menu bar shows the
# app's name and Finder can open files with it ("このアプリケーションで開く").
#
#   poc/gpui/script/bundle-macos.sh [cargo build options]
#   open poc/gpui/target/release/UmeEditorPoC.app
#
# Options go to `cargo build --release`, for example `--no-default-features`
# to precompile the Metal shaders (needs Xcode's Metal Toolchain).
set -eu

cd "$(dirname "$0")/.."
cargo build --release "$@"

app=target/release/UmeEditorPoC.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/ume-poc-gpui "$app/Contents/MacOS/"

# LSHandlerRank "Alternate": the app shows up under "Open With" for text
# files without becoming the default app for anything.
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>ume_editor PoC</string>
	<key>CFBundleDisplayName</key>
	<string>ume_editor UI 試作</string>
	<key>CFBundleIdentifier</key>
	<string>io.github.stofu1234.ume-editor.poc-gpui</string>
	<key>CFBundleExecutable</key>
	<string>ume-poc-gpui</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.0.0</string>
	<key>CFBundleVersion</key>
	<string>0</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Text</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Alternate</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.text</string>
				<string>public.data</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
PLIST

# Ad-hoc signature, so that macOS runs the bundle as one app.
codesign --force --sign - "$app"

# Tell Launch Services about the bundle, so that "Open With" lists it without
# launching it first.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"

echo "$app"
