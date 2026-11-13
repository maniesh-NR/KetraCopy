# =============================================================================
# KetraCopy context-menu uninstaller (PowerShell 2.0 compatible).
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File context-menu-uninstall.ps1 `
#       -Scope machine     (default; elevated)
#   ... -Scope user
#
# Removes ONLY the KetraCopy* keys written by context-menu-install.ps1 from
# *\shell, Directory\shell and Directory\Background\shell. Safe to run even
# when nothing is installed.
#
# NOTE: duplicated in integrations\windows\context-menu-uninstall.ps1. Keep in sync.
# =============================================================================

param(
    [ValidateSet("user", "machine")]
    [string]$Scope = "machine"
)

$ErrorActionPreference = "Continue"

if ($Scope -eq "machine") {
    $classes = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Classes", $true)
}
else {
    $classes = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes", $true)
}
if ($classes -eq $null) {
    Write-Warning "Cannot open the Classes hive for scope '$Scope'; nothing removed."
    exit 1
}

$leaves = @("KetraCopyCopy", "KetraCopyMove", "KetraCopyTest")
$parents = @("*\shell", "Directory\shell", "Directory\Background\shell")

foreach ($shellPath in $parents) {
    $shellKey = $classes.OpenSubKey($shellPath, $true)
    if ($shellKey -eq $null) {
        continue
    }
    foreach ($leaf in $leaves) {
        $names = $shellKey.GetSubKeyNames()
        if ($names -contains $leaf) {
            $shellKey.DeleteSubKeyTree($leaf)
            Write-Host "  removed  $shellPath\$leaf"
        }
    }
    $shellKey.Close()
}

$classes.Close()

Write-Host ""
Write-Host "KetraCopy context menu removed (scope: $Scope)."
exit 0
