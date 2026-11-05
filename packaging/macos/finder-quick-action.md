# Finder Quick Action (macOS 10.15+) for KetraCopy

Finder "Quick Actions" (formerly Services) let you right-click any selection
and run **KetraCopy: Copy to Folder...** / **KetraCopy: Move to Folder...**.
The action asks for a destination folder and then runs the KetraCopy CLI:

```
ketra copy  <selected items...> -d <chosen folder>
ketra move  <selected items...> -d <chosen folder>
```

Two ways to get it:

---

## Option A — generated (automated)

```console
$ ./packaging/macos/make-quick-action.sh          # installs Copy + Move
$ ./packaging/macos/make-quick-action.sh copy     # only Copy
```

The script writes `~/Library/Services/KetraCopy-{copy,move}.workflow`
(Automator workflow bundles) and validates them with `plutil -lint`.

**What is automatic vs manual — being honest:**

* ✅ Automatic: bundle creation, plist generation, `plutil` syntax
  validation, `pbs -update` refresh.
* ⚠️ Semi-automatic: the `document.wflow` is hand-written to match
  Automator's format (one *Run Shell Script* action, input = arguments).
  macOS versions differ slightly in how strictly Automator imports it. If
  Automator opens the workflow and shows the shell step with the script
  intact, you are done.
* ❌ Manual fallback: if the generated workflow ever fails to load, build it
  by hand in ~2 minutes (below) and delete the generated one.

The action will not appear until macOS notices it:

1. Run `/System/Library/CoreServices/pbs -update` (the script does this), or
2. log out and back in, and
3. enable it under **System Settings → Privacy & Security → Extensions →
   Finder** (macOS 13+: **General → Login Items & Extensions → Finder**).

---

## Option B — manual (Automator, 2 minutes)

1. Open **Automator** → File → New → **Quick Action**.
2. At the top of the workflow area set:
   * *Workflow receives current* → **files or folders** *in*
   * **Finder**
   * (leave "Output replaces selected text" unchecked)
3. Drag the **Run Shell Script** action into the workflow.
4. Set *Shell*: `/bin/bash`, *Pass input*: **as arguments**.
5. Paste:

   ```bash
   KETRA=""
   for c in /usr/local/bin/ketra /opt/homebrew/bin/ketra "$HOME/.cargo/bin/ketra" "$HOME/bin/ketra"; do
     if [ -x "$c" ]; then KETRA="$c"; break; fi
   done
   if [ -z "$KETRA" ]; then
     osascript -e 'display alert "KetraCopy" message "ketra CLI not found (looked in /usr/local/bin, /opt/homebrew/bin, ~/.cargo/bin, ~/bin)."'
     exit 1
   fi
   DEST=$(osascript -e 'POSIX path of (choose folder with prompt "KetraCopy: choose destination folder")') || exit 0
   exec "$KETRA" copy "$@" -d "$DEST"
   ```

   (For the Move action, change `copy` to `move` in the last line.)
6. Save as **KetraCopy: Copy to Folder...** (and repeat with `move`).
   Automator stores it in `~/Library/Services/KetraCopy Copy to Folder....workflow`.

---

## Prerequisites

* The `ketra` CLI on disk. Recommended (after installing KetraCopy.app to
  /Applications):

  ```console
  $ sudo mkdir -p /usr/local/bin
  $ sudo ln -sf /Applications/KetraCopy.app/Contents/MacOS/ketra /usr/local/bin/ketra
  ```

  The embedded script also checks `/opt/homebrew/bin`, `~/.cargo/bin` and
  `~/bin`, or you can hardcode another path.

## Uninstall

```console
$ rm -rf ~/Library/Services/KetraCopy-copy.workflow ~/Library/Services/KetraCopy-move.workflow
$ /System/Library/CoreServices/pbs -update
```

## Notes

* A console window is not shown; errors surface via `osascript` alerts. For
  full progress/verification output use the terminal or the GUI.
* Multi-selection works: Finder passes every selected item as an argument,
  and `ketra copy a b c -d DEST` handles the batch in one verified run.
