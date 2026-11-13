# =============================================================================
# KetraCopy context-menu installer (PowerShell 2.0 compatible).
#
# Usage (from an elevated prompt for machine scope):
#   powershell -NoProfile -ExecutionPolicy Bypass -File context-menu-install.ps1 `
#       -Path "C:\Program Files\KetraCopy" -Scope machine
#   ... -Scope user            (no elevation needed; writes HKCU)
#
# Registers, under HKLM\SOFTWARE\Classes (machine) or HKCU\Software\Classes (user):
#   *\shell\KetraCopyCopy\command            helper copy "%1"
#   *\shell\KetraCopyMove\command            helper move "%1"
#   *\shell\KetraCopyTest\command            helper test "%1"
#   Directory\shell\KetraCopyCopy\command    helper copy "%1"        (folders)
#   Directory\Background\shell\...\command   helper <action> "%V"    (folder background)
#
# The helper (context-menu.ps1) shows a destination-folder picker and runs
# ketra copy|move|test "<item>" -d "<folder>". Re-running this script is
# idempotent (it recreates the keys). Uninstall with context-menu-uninstall.ps1.
#
# PS 2.0 safe: no $PSScriptRoot, no ternary, no && chains; registry writes use
# the .NET [Microsoft.Win32.Registry] API (avoids PowerShell wildcard problems
# with the literal "*" key name).
#
# NOTE: duplicated in integrations\windows\context-menu-install.ps1. Keep in sync.
# =============================================================================

param(
    [string]$Path = "$Env:ProgramFiles\KetraCopy",
    [ValidateSet("user", "machine")]
    [string]$Scope = "machine",
    [string]$KetraExe = ""
)

$ErrorActionPreference = "Stop"

$helper = Join-Path $Path "context-menu.ps1"
if (-not (Test-Path -LiteralPath $helper)) {
    throw "context-menu.ps1 not found in '$Path'. Reinstall KetraCopy or pass -Path."
}
if ($KetraExe -eq "") {
    $KetraExe = Join-Path $Path "ketra.exe"
}

$psExe = Join-Path $Env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
if (-not (Test-Path -LiteralPath $psExe)) {
    throw "Windows PowerShell not found at '$psExe'."
}

if ($Scope -eq "machine") {
    $classes = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Classes", $true)
}
else {
    $classes = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes", $true)
}
if ($classes -eq $null) {
    throw "Cannot open the Classes hive for scope '$Scope'. Run elevated for -Scope machine."
}

# leaf registry key, menu caption, ketra sub-command
$actions = @(
    ,@("KetraCopyCopy", "KetraCopy: Copy to Folder...", "copy")
    ,@("KetraCopyMove", "KetraCopy: Move to Folder...", "move")
    ,@("KetraCopyTest", "KetraCopy: Test (Verify)...", "test")
)

# shell parent key, Explorer argument placeholder
$parents = @(
    ,@("*\shell", "%1")                       # any selected file or folder
    ,@("Directory\shell", "%1")               # right-click on a folder
    ,@("Directory\Background\shell", "%V")    # right-click inside a folder window
)

foreach ($parent in $parents) {
    $shellPath = $parent[0]
    $placeholder = $parent[1]
    foreach ($entry in $actions) {
        $leaf = $entry[0]
        $caption = $entry[1]
        $ketraCmd = $entry[2]

        $menuKey = $classes.CreateSubKey("$shellPath\$leaf")
        $menuKey.SetValue("", $caption)
        if (Test-Path -LiteralPath $KetraExe) {
            $menuKey.SetValue("Icon", $KetraExe)
        }
        $cmdKey = $menuKey.CreateSubKey("command")
        $command = '"' + $psExe + '" -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + `
            $helper + '" ' + $ketraCmd + ' "' + $placeholder + '"'
        $cmdKey.SetValue("", $command)
        $cmdKey.Close()
        $menuKey.Close()
        Write-Host "  added  $shellPath\$leaf"
    }
}

$classes.Close()

Write-Host ""
Write-Host "KetraCopy context menu installed (scope: $Scope)."
Write-Host "Open Explorer windows may need a refresh (or: taskkill /f /im explorer.exe && start explorer.exe)."
exit 0
