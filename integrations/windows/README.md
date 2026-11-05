# Windows Explorer context-menu integration (KetraCopy)

These PowerShell scripts register **KetraCopy: Copy to Folder...**,
**KetraCopy: Move to Folder...** and **KetraCopy: Test (Verify)...** in the
Windows Explorer right-click menu for files, folders and the folder
background. The menu entry opens `context-menu.ps1`, which shows a
destination-folder picker and runs:

```
ketra copy|move|test "<selected item>" -d "<picked folder>"
```

(The ketra CLI requires `-d` for `copy`, `move` **and** `test` — see
`crates/ketra-cli/src/args.rs`. For `test`, the picked folder is used as the
scratch destination.)

> NOTE: identical copies of these scripts live in
> `packaging/windows/` where the Inno Setup installer
> (`packaging/windows/ketracopy.iss`) bundles and runs them post-install.
> Keep both copies in sync.

## Install (manual)

```powershell
# Per-machine (all users) — run from an elevated PowerShell:
powershell -NoProfile -ExecutionPolicy Bypass -File .\context-menu-install.ps1 `
    -Path "C:\Program Files\KetraCopy" -Scope machine

# Per-user (no elevation needed):
powershell -NoProfile -ExecutionPolicy Bypass -File .\context-menu-install.ps1 `
    -Path "C:\Program Files\KetraCopy" -Scope user
```

Parameters:

| Parameter   | Meaning                                                          |
|-------------|------------------------------------------------------------------|
| `-Path`     | Install directory containing `ketra.exe` + `context-menu.ps1`    |
| `-Scope`    | `machine` (HKLM\SOFTWARE\Classes) or `user` (HKCU\Software\Classes) |
| `-KetraExe` | Optional explicit path to `ketra.exe` (defaults to `<Path>\ketra.exe`) |

## Uninstall

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\context-menu-uninstall.ps1 -Scope machine
# or: -Scope user
```

Removes only the `KetraCopy*` keys written by the installer; safe to run even
if nothing is installed.

## What gets written

For scope `machine`, under `HKLM\SOFTWARE\Classes` (for `user`:
`HKCU\Software\Classes`) — `HKCR` is the merged view of both:

| Key                                     | Default value (menu caption)      | `command`                                                          |
|-----------------------------------------|-----------------------------------|---------------------------------------------------------------------|
| `*\shell\KetraCopyCopy`                 | KetraCopy: Copy to Folder...      | `powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "<Path>\context-menu.ps1" copy "%1"` |
| `*\shell\KetraCopyMove`                 | KetraCopy: Move to Folder...      | `... context-menu.ps1 move "%1"`                                   |
| `*\shell\KetraCopyTest`                 | KetraCopy: Test (Verify)...       | `... context-menu.ps1 test "%1"`                                   |
| `Directory\shell\KetraCopy{Copy,Move,Test}` | same                          | same (`%1`)                                                         |
| `Directory\Background\shell\KetraCopy{...}` | same                          | same but `%V` (folder currently open)                               |

Each key also gets `Icon` = `<Path>\ketra.exe`.

## Behaviour notes

* **Multi-select:** classic (non-`ExplorerCommand`) registry verbs are invoked
  once *per selected item*, so selecting 5 files opens 5 pickers. That is the
  accepted trade-off for PS-2.0/Win7 compatibility; use the GUI for one-shot
  batch jobs (`ketra copy <many sources> -d <dest>`).
* **Folder background:** right-clicking empty space in a folder passes the
  current folder as the source (`%V`).
* A console window appears showing ketra's live progress and closes when the
  transfer finishes (errors/exit codes 1/2/3/4 are visible in that window).
* `powershell.exe` (Windows PowerShell, present since Windows 7) is used
  explicitly; the scripts are **PS 2.0 compatible**: no `$PSScriptRoot`, no
  ternaries, no `&&` chains, registry access via the .NET
  `[Microsoft.Win32.Registry]` API (also avoids PowerShell wildcard issues
  with the literal `*` key).
* If the menu does not refresh immediately, restart Explorer
  (`taskkill /f /im explorer.exe && start explorer.exe`).

## Troubleshooting

* **"running scripts is disabled"** — the registered command already passes
  `-ExecutionPolicy Bypass`; if you call the scripts manually, add it too.
* **Menu entries missing** — check the right hive for your scope
  (`HKLM:\SOFTWARE\Classes` vs `HKCU:\Software\Classes`) and that `ketra.exe`
  exists at `-Path`.
* **Wrong install path after moving** — re-run the installer with the new
  `-Path`; it rewrites the keys (idempotent).
