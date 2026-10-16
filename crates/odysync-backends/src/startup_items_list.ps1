# Lists Windows startup entries as JSON. Read-only.
$ErrorActionPreference = 'SilentlyContinue'

function Get-ApprovalMap([string]$approvedPath) {
    $map = @{}
    if (Test-Path $approvedPath) {
        $key = Get-Item $approvedPath
        foreach ($valueName in $key.GetValueNames()) {
            $bytes = $key.GetValue($valueName)
            if ($bytes -is [byte[]] -and $bytes.Length -gt 0) {
                $map[$valueName] = (($bytes[0] % 2) -eq 0)
            }
        }
    }
    return $map
}

$result = New-Object System.Collections.ArrayList

$runKeys = @(
    @{ Path = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
       Approved = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' },
    @{ Path = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Run'
       Approved = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run' }
)

foreach ($entry in $runKeys) {
    if (-not (Test-Path $entry.Path)) { continue }
    $approved = Get-ApprovalMap $entry.Approved
    $key = Get-Item $entry.Path
    foreach ($valueName in $key.GetValueNames()) {
        if ([string]::IsNullOrEmpty($valueName)) { continue }
        $enabled = $true
        if ($approved.ContainsKey($valueName)) { $enabled = $approved[$valueName] }
        [void]$result.Add([PSCustomObject]@{
            Name     = $valueName
            Command  = [string]$key.GetValue($valueName)
            Location = $entry.Path
            Enabled  = $enabled
        })
    }
}

$startupFolders = @(
    [Environment]::GetFolderPath('Startup'),
    (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Startup')
)
foreach ($folder in $startupFolders) {
    if ([string]::IsNullOrEmpty($folder) -or -not (Test-Path $folder)) { continue }
    foreach ($file in Get-ChildItem -LiteralPath $folder -File) {
        # `.disabled` is the marker this app writes when turning an entry off.
        $isDisabled = $file.Name -like '*.disabled'
        $displayName = $file.BaseName -replace '\.disabled$', ''
        [void]$result.Add([PSCustomObject]@{
            Name     = $displayName
            Command  = $file.FullName
            Location = $folder
            Enabled  = (-not $isDisabled)
        })
    }
}

@($result | Sort-Object Name) | ConvertTo-Json -Depth 3 -Compress
