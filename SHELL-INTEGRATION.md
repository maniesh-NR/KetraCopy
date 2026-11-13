# Shell integration — right-click menus, clipboard, keyboard shortcuts

KetraCopy 1.2.0 integrates with the **native cut / copy / paste / move**
flows of the desktop file managers. Two directions:

1. **KetraCopy commands from the shell** — right-click menus and keyboard
   shortcuts that run the engine on the current selection.
2. **KetraCopy as the paste engine** — `ketra clip paste` reads whatever the
   *native* file manager put on the clipboard (including its cut state) and
   transfers it with the engine: accelerated, verified, resumable.

Install everything for the current platform with:

```sh
ketra shell install          # default set for the detected OS
ketra shell install nemo shortcuts   # Linux: specific targets
ketra shell uninstall
ketra shell status
```

(Or press **Install** in the GUI's Settings tab.)

---

## Windows (Explorer)

`ketra shell install` writes per-user registry verbs under
`HKCU\Software\Classes` (no administrator rights needed) and two helpers
into `%LOCALAPPDATA%\KetraCopy`:

| Menu entry | Command |
|---|---|
| KetraCopy: Copy | `ketra clip put "%1"` |
| KetraCopy: Cut | `ketra clip put --cut "%1"` |
| KetraCopy: Paste here | `ketra clip paste -d "%1" -y` (folder + background `%V`) |
| KetraCopy: Copy to Folder… / Move to Folder… / Test | folder-picker helper (`ketra-pick.ps1`) → `ketra copy|move|test` |

* **Clipboard format**: `CF_HDROP` (`DROPFILES`, UTF-16) +
  `Preferred DropEffect` (1 = copy, 2 = cut) — identical to what Explorer
  itself puts there, so **Ctrl+V keeps working everywhere** and KetraCopy's
  `clip paste` understands Explorer's own Ctrl+C / Ctrl+X.
* **Keyboard shortcuts**: the installer writes
  `%LOCALAPPDATA%\KetraCopy\KetraCopy.ahk` (AutoHotkey **v2**). After
  installing AutoHotkey from autohotkey.com, double-click it (or put a
  shortcut into `shell:startup`):
  * `Ctrl+Shift+C` — put the selected Explorer items (copy)
  * `Ctrl+Shift+X` — put the selected Explorer items (cut)
  * `Ctrl+Shift+V` — paste the clipboard through the engine into the
    *active* Explorer folder (resolves the folder via Shell COM)
* **Multi-select note**: legacy registry verbs receive one item per run;
  for multi-file operations use the clipboard flow (Ctrl+C → KetraCopy
  paste) or the AutoHotkey shortcuts. `MultiSelectModel=Player` is set so
  the menu still appears on multi-selections.
* Legacy PowerShell installers for machine-wide registration remain in
  `integrations/windows/` and `packaging/windows/` (Inno Setup).

## Linux (Mint / Ubuntu / Fedora / KDE)

Targets (`ketra shell install <target>`): `nemo`, `nautilus`, `caja`,
`dolphin`, `thunar`, `pick` (destination picker into `~/.local/bin`),
`shortcuts` (Cinnamon), `all`.

| File manager | Mechanism | Entries |
|---|---|---|
| Nemo | actions in `~/.local/share/nemo/actions/` | Copy to Folder… / Cut / Move to Folder… / Test |
| Nemo / Nautilus / Caja | scripts in `~/.local/share/<fm>/scripts/` | Copy, Move to Folder…, Test, **Paste Here** |
| Dolphin | service menu in `~/.local/share/kio/servicemenus/` | submenu KetraCopy: Copy to Folder… / Cut / Move to Folder… / Test / **Paste here** |
| Thunar | `~/.config/Thunar/ketra-uca-snippet.xml` (merge into `uca.xml`) | Copy to Folder… / Move to Folder… |
| Cinnamon shortcuts | `gsettings` custom keybinding | **Super+V** → `ketra-paste-smart` |

* **Clipboard formats written by `ketra clip put`**: `text/uri-list` +
  `text/x-special/gnome-copied-files` (`copy`/`cut` first line) +
  `application/x-kde-cutselection`. The selection owner is a detached
  `ketra __clip-server` daemon (X11 only, `dlopen` libX11 at runtime — no
  hard dependency); it exits when another app takes the selection.
* **Clipboard formats read by `ketra clip paste`**: the same set — so
  copying/cutting inside Nemo/Nautilus/Dolphin and then pasting through
  KetraCopy preserves the cut state.
* **Paste Here** scripts resolve the target folder from the file manager's
  `*_SCRIPT_CURRENT_URI` environment variable (or the single selected
  folder).
* **Copy to Folder… / Move to Folder…** scripts (Nautilus/Caja) route
  through the `ketra-pick` destination picker; the picker honours
  `KETRA_DEST=<dir>` to skip the dialog (scripting / headless use) and
  `KETRA_PICKER=<zenity|kdialog|yad>` to force a backend.
* **Super+V** resolves the active Nemo window title conservatively
  (exact folder-name match under `$HOME`); otherwise it opens a folder
  picker (`zenity`/`kdialog`/`yad`).
* **Wayland note**: the X11 selection daemon cannot run on native Wayland
  sessions; the right-click integrations are unaffected there. Run the
  file managers under XWayland for clipboard commands.

## macOS (Finder)

`ketra shell install` writes Automator Quick Actions to
`~/Library/Services`:

| Service | Receives | Action |
|---|---|---|
| Copy with KetraCopy | files/folders | `ketra clip put "$@"` |
| Cut with KetraCopy | files/folders | `ketra clip put --cut "$@"` |
| Move with KetraCopy… | files/folders | folder chooser → `ketra move … -d` |
| Paste with KetraCopy | nothing | folder chooser → `ketra clip paste -d` |

* Finder's pasteboard is reached through AppleScript (`«class furl`»);
  items copied with KetraCopy paste natively with Cmd+V.
* Finder's own Cmd+X state is not scriptable — KetraCopy's cut flag is
  stored in a side-channel (`~/.config/ketra/clipboard-mode`, 1 h expiry)
  honoured by `clip paste`.
* **Keyboard shortcuts**: System Settings → Keyboard → Keyboard
  Shortcuts → Services → Files and Folders → check the KetraCopy items
  and assign shortcuts (e.g. ⇧⌘C / ⇧⌘X / ⇧⌘V).

## Format reference (for interoperability testing)

| Platform | Write | Cut marker |
|---|---|---|
| Windows | `CF_HDROP` (`DROPFILES`, UTF-16, double-NUL list) | `Preferred DropEffect` bit 1 (MOVE) |
| Linux/X11 | `text/uri-list` (CRLF) + `text/x-special/gnome-copied-files` | gnome header line `cut` / KDE cutselection `1` |
| macOS | Finder pasteboard via AppleScript `POSIX file` list | side-channel mode file |

The codecs live in `crates/ketra-cli/src/clip.rs` and are unit-tested on
every platform (DROPFILES parse, URI escaping, gnome header, DropEffect).
