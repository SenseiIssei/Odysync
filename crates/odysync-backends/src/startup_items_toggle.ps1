# Switches one startup entry on or off. Expects $name, $location and $enable
# to be defined by the caller, each already quoted as a PowerShell literal.
$ErrorActionPreference = 'Stop'

if ($location -like 'HKCU:*' -or $location -like 'HKLM:*') {
    $hive = if ($location -like 'HKLM:*') { 'HKLM:' } else { 'HKCU:' }
    $approvedPath = "$hive\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
    if (-not (Test-Path $approvedPath)) {
        New-Item -Path $approvedPath -Force | Out-Null
    }
    # First byte even = enabled, odd = disabled. The remaining 11 bytes are a
    # timestamp Windows does not require us to fill in.
    $first = if ($enable) { 0x02 } else { 0x03 }
    $blob = [byte[]]($first,0,0,0,0,0,0,0,0,0,0,0)
    Set-ItemProperty -LiteralPath $approvedPath -Name $name -Value $blob -Type Binary
} else {
    $enabledPath  = Join-Path $location "$name.lnk"
    $disabledPath = Join-Path $location "$name.lnk.disabled"
    if ($enable) {
        if (Test-Path -LiteralPath $disabledPath) {
            Rename-Item -LiteralPath $disabledPath -NewName "$name.lnk" -Force
        } elseif (-not (Test-Path -LiteralPath $enabledPath)) {
            throw "No startup entry found for $name"
        }
    } else {
        if (Test-Path -LiteralPath $enabledPath) {
            Rename-Item -LiteralPath $enabledPath -NewName "$name.lnk.disabled" -Force
        } elseif (-not (Test-Path -LiteralPath $disabledPath)) {
            throw "No startup entry found for $name"
        }
    }
}
Write-Output "ODYSYNC_OK"
