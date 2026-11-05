#!/usr/bin/env bash
# =============================================================================
# packaging/macos/make-quick-action.sh — install Finder Quick Actions that
# hand the current Finder selection to the KetraCopy CLI.
#
# Usage:
#   ./make-quick-action.sh            # install both (Copy + Move)
#   ./make-quick-action.sh copy       # only the Copy action
#   ./make-quick-action.sh move       # only the Move action
#
# Creates Automator workflow bundles in ~/Library/Services:
#   KetraCopy-copy.workflow   "KetraCopy: Copy to Folder..."
#   KetraCopy-move.workflow   "KetraCopy: Move to Folder..."
#
# Each workflow contains a single "Run Shell Script" action (input passed as
# arguments, /bin/bash). The embedded script asks for a destination folder
# via `osascript -e 'choose folder'` and then runs:
#     ketra copy|move "$@" -d "$DEST"
#
# HONESTY NOTES:
# - The document.wflow / Info.plist here are generated (not authored in the
#   Automator GUI). `plutil -lint` below guarantees they are *valid plists*,
#   but cannot guarantee Automator's importer accepts every detail. If the
#   service misbehaves, build it manually in 2 minutes — the exact steps are
#   in packaging/macos/finder-quick-action.md.
# - After installing, macOS may need a moment (or the pbs refresh below,
#   or a logout) before the actions appear under right-click > Quick Actions.
# - Enable them under: System Settings > Privacy & Security > Extensions >
#   Finder (or General > Login Items & Extensions on macOS 13+).
#
# macOS only: exits with a clear error on Linux/Windows.
# =============================================================================
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OS="$(uname -s)"
if [ "$OS" != "Darwin" ]; then
    echo "ERROR: make-quick-action.sh must run on macOS (uname -s = '$OS')." >&2
    echo "The generated workflow bundles are Apple-only." >&2
    exit 1
fi

WHICH="${1:-both}"
case "$WHICH" in
    copy|move|both) ;;
    *)
        echo "usage: $0 [copy|move|both]" >&2
        exit 2
        ;;
esac

SERVICES_DIR="$HOME/Library/Services"
mkdir -p "$SERVICES_DIR"

install_workflow() {
    action="$1" # copy | move
    if [ "$action" = "copy" ]; then
        label="KetraCopy: Copy to Folder..."
    else
        label="KetraCopy: Move to Folder..."
    fi
    wf="$SERVICES_DIR/KetraCopy-$action.workflow"
    echo "==> $wf"

    rm -rf "$wf"
    mkdir -p "$wf/Contents"

    # ---- Contents/Info.plist: registers the service with Finder -----------
    cat > "$wf/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSBackgroundColorName</key>
			<string>background</string>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>$label</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>public.item</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
PLIST

    # ---- Contents/document.wflow: one "Run Shell Script" action -----------
    # Input method 1 = "as arguments" (the selection arrives in "$@").
    # The embedded script deliberately avoids XML-hostile characters
    # (no <, >, &) so it can be inlined verbatim.
    cat > "$wf/Contents/document.wflow" <<'WFLOW'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.path</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.path</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>KETRA=""
for c in /usr/local/bin/ketra /opt/homebrew/bin/ketra "$HOME/.cargo/bin/ketra" "$HOME/bin/ketra"; do
  if [ -x "$c" ]; then KETRA="$c"; break; fi
done
if [ -z "$KETRA" ]; then
  osascript -e 'display alert "KetraCopy" message "ketra CLI not found (looked in /usr/local/bin, /opt/homebrew/bin, ~/.cargo/bin, ~/bin)."'
  exit 1
fi
DEST=$(osascript -e 'POSIX path of (choose folder with prompt "KetraCopy __ACTION__: choose destination folder")') || exit 0
exec "$KETRA" __ACTION__ "$@" -d "$DEST"
</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/bash</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>1A2B3C4D-5E6F-4A0B-8C1D-2E3F4A5B6C7D</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>KetraCopy</string>
				</array>
				<key>OutputUUID</key>
				<string>7E8F9A0B-1C2D-4E3F-8A5B-6C7D8E9F0A1B</string>
				<key>UUID</key>
				<string>3C4D5E6F-7A8B-4C9D-8E1F-2A3B4C5D6E7F</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict>
					<key>0</key>
					<dict>
						<key>default value</key>
						<integer>0</integer>
						<key>name</key>
						<string>inputMethod</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>0</string>
					</dict>
					<key>1</key>
					<dict>
						<key>default value</key>
						<false/>
						<key>name</key>
						<string>CheckedForUserDefaultShell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>1</string>
					</dict>
					<key>2</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>source</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>2</string>
					</dict>
					<key>3</key>
					<dict>
						<key>default value</key>
						<string></string>
						<key>name</key>
						<string>COMMAND_STRING</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>3</string>
					</dict>
					<key>4</key>
					<dict>
						<key>default value</key>
						<string>/bin/sh</string>
						<key>name</key>
						<string>shell</string>
						<key>required</key>
						<string>0</string>
						<key>type</key>
						<string>0</string>
						<key>uuid</key>
						<string>4</string>
					</dict>
				</dict>
				<key>isViewVisible</key>
				<integer>1</integer>
				<key>location</key>
				<string>309.000000:253.000000</string>
				<key>nibPath</key>
				<string>/System/Library/Automator/Run Shell Script.action/Contents/Resources/Base.lproj/main.nib</string>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
WFLOW

    # Fill in the action word (copy / move) in the embedded script.
    # Replacement text is a plain word, so sed is safe here.
    sed -i '' "s/__ACTION__/$action/g" "$wf/Contents/document.wflow"

    # ---- validate ----------------------------------------------------------
    if plutil -lint "$wf/Contents/Info.plist" "$wf/Contents/document.wflow"; then
        echo "    plutil: OK"
    else
        echo "    WARNING: generated plists failed plutil -lint." >&2
        echo "    Create the Quick Action manually instead - see" >&2
        echo "    packaging/macos/finder-quick-action.md." >&2
        rm -rf "$wf"
        return 1
    fi
}

if [ "$WHICH" = "copy" ] || [ "$WHICH" = "both" ]; then
    install_workflow copy
fi
if [ "$WHICH" = "move" ] || [ "$WHICH" = "both" ]; then
    install_workflow move
fi

# Ask launchd's services cache to rescan (best effort; may need logout).
if [ -x "/System/Library/CoreServices/pbs" ]; then
    /System/Library/CoreServices/pbs -update 2>/dev/null || true
fi

echo ""
echo "Installed. If the actions do not appear in Finder right away:"
echo "  1. System Settings > Privacy & Security > Extensions > Finder - enable them"
echo "  2. or log out/in"
echo "If Automator refuses to open a workflow, build it manually:"
echo "  see packaging/macos/finder-quick-action.md"
exit 0
