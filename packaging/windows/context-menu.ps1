# =============================================================================
# KetraCopy Explorer context-menu helper (PowerShell 2.0 compatible).
#
# Called by the registry verbs written by context-menu-install.ps1, e.g.:
#   powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden `
#       -File "C:\Program Files\KetraCopy\context-menu.ps1" copy "%1"
#
# Shows a destination-folder picker, then runs the ketra CLI:
#   ketra copy|move|test "<target>" -d "<picked folder>"
#
# (The ketra CLI requires -d for copy, move AND test - see
#  crates/ketra-cli/src/args.rs; the picked folder doubles as the scratch
#  destination for "test".)
#
# Windows 7 notes: PS 2.0 safe - no $PSScriptRoot (PS 3.0+), no ternary,
# no && / || chains. A console window opens showing ketra's progress and
# closes when the job finishes; use the GUI for pause/resume dialogs.
#
# NOTE: this file is duplicated in integrations\windows\context-menu.ps1
#       (canonical copy for manual installs). Keep both in sync.
# =============================================================================

param(
    [Parameter(Mandatory = $true)]
    [string]$Action,
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [string]$KetraExe = ""
)

$ErrorActionPreference = "Stop"

if ($Action -ne "copy" -and $Action -ne "move" -and $Action -ne "test") {
    $Action = "copy"
}

Add-Type -AssemblyName System.Windows.Forms

# ---- locate ketra.exe -------------------------------------------------------
$exe = ""
if ($KetraExe -ne "" -and (Test-Path -LiteralPath $KetraExe)) {
    $exe = $KetraExe
}
if ($exe -eq "") {
    # Same directory as this script ($PSScriptRoot is PS 3.0+, so use this).
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $candidate = Join-Path $scriptDir "ketra.exe"
    if (Test-Path -LiteralPath $candidate) { $exe = $candidate }
}
if ($exe -eq "" -and (Test-Path -LiteralPath "$Env:ProgramFiles\KetraCopy\ketra.exe")) {
    $exe = "$Env:ProgramFiles\KetraCopy\ketra.exe"
}
if ($exe -eq "") {
    $found = Get-Command "ketra.exe" -ErrorAction SilentlyContinue
    if ($found -ne $null) { $exe = $found.Path }
}
if ($exe -eq "") {
    $msg = "KetraCopy: ketra.exe was not found." + [Environment]::NewLine + `
        "Reinstall KetraCopy, or update the context-menu entry with -KetraExe."
    [void][System.Windows.Forms.MessageBox]::Show($msg, "KetraCopy")
    exit 1
}

# ---- destination folder picker ----------------------------------------------
$picker = New-Object System.Windows.Forms.FolderBrowserDialog
$picker.Description = "KetraCopy " + $Action + ": choose the destination folder"
$picker.ShowNewFolderButton = $true
$dialogResult = $picker.ShowDialog()
if ($dialogResult -ne [System.Windows.Forms.DialogResult]::OK) {
    exit 0
}
$dest = $picker.SelectedPath
if ($dest -eq "") {
    exit 0
}

# ---- run ketra ---------------------------------------------------------------
# UseShellExecute = $true gives ketra its own console window with live
# progress; it closes automatically when the transfer ends.
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $exe
$psi.Arguments = '{0} "{1}" -d "{2}"' -f $Action, $Target, $dest
$psi.UseShellExecute = $true
[void][System.Diagnostics.Process]::Start($psi)

exit 0
