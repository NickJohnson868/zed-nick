#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$InstallDirectory = $PSScriptRoot,
    [switch]$ApplyAfterExit
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$packageRoot = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
$installRoot = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\')
if ($installRoot.Length -le [IO.Path]::GetPathRoot($installRoot).Length) {
    throw 'InstallDirectory must be an application directory, not a drive root.'
}
$manifest = Get-Content -LiteralPath (Join-Path $packageRoot 'bundle-manifest.json') -Raw | ConvertFrom-Json

function Resolve-BundlePath([string]$Root, [string]$Relative) {
    if ([IO.Path]::IsPathRooted($Relative) -or @($Relative.Replace('\','/').Split('/') | Where-Object { $_ -in @('..','') }).Count) {
        throw "Invalid bundle path: $Relative"
    }
    $resolved = [IO.Path]::GetFullPath((Join-Path $Root $Relative))
    if (!$resolved.StartsWith($Root+'\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Bundle path is outside its directory: $Relative"
    }
    return $resolved
}

function Get-TargetProcess {
    Get-Process -Name Zed -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.Equals((Join-Path $installRoot 'Zed.exe'), [StringComparison]::OrdinalIgnoreCase)
    }
}

foreach ($file in $manifest.files) {
    $source = Resolve-BundlePath $packageRoot $file.path
    if (!(Test-Path -LiteralPath $source -PathType Leaf) -or
        (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $file.sha256) {
        throw "Package integrity check failed: $($file.path)"
    }
    [void](Resolve-BundlePath $installRoot $file.target)
}

$changed = @($manifest.files | Where-Object {
    $destination = Resolve-BundlePath $installRoot $_.target
    !(Test-Path -LiteralPath $destination -PathType Leaf) -or
        (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $_.sha256
})
if ($changed.Count -and @(Get-TargetProcess).Count -and !$ApplyAfterExit) {
    $pendingRoot = Join-Path $installRoot ('pending-update\'+(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N'))
    foreach ($file in $manifest.files) {
        $destination = Resolve-BundlePath $pendingRoot $file.path
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
        Copy-Item -LiteralPath (Resolve-BundlePath $packageRoot $file.path) -Destination $destination -Force
    }
    foreach ($name in @('Update-Zed.ps1','bundle-manifest.json','settings-patch.json','UPDATE-README.txt','另一台机器更新提示词.txt')) {
        $source = Join-Path $packageRoot $name
        if (Test-Path -LiteralPath $source -PathType Leaf) {
            Copy-Item -LiteralPath $source -Destination (Join-Path $pendingRoot $name) -Force
        }
    }
    $arguments = @('-NoLogo','-NoProfile','-File',('"'+(Join-Path $pendingRoot 'Update-Zed.ps1')+'"'),'-InstallDirectory',('"'+$installRoot+'"'),'-ApplyAfterExit')
    @{status='pending';stage=$pendingRoot;commit=$manifest.commit;automatic_restart=$false} |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $installRoot 'pending-update.json') -Encoding utf8
    $helper = Start-Process -FilePath (Get-Process -Id $PID).Path -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $pendingRoot 'update.log') -RedirectStandardError (Join-Path $pendingRoot 'update-error.log')
    $helper.Id | Set-Content -LiteralPath (Join-Path $pendingRoot 'helper.pid') -Encoding utf8
    Write-Host "Update staged: $pendingRoot"
    Write-Host 'Zed was left running. The helper will apply files after Zed exits normally; it never launches or restarts Zed.'
    return
}

if ($ApplyAfterExit) {
    while (@(Get-TargetProcess).Count) { Start-Sleep -Seconds 1 }
}
$backupRoot = Join-Path $installRoot ('update-backups\'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
foreach ($plugin in $manifest.plugins) {
    if ($plugin.id -notmatch '^[a-z0-9-]+$') { throw 'Invalid plugin identity' }
    foreach ($relative in @("portable-data/extensions/installed/$($plugin.id)", "portable-data/extensions/work/$($plugin.id)", "portable-data/extensions/work/$($plugin.id)/bin")) {
        $path = Resolve-BundlePath $installRoot $relative
        $item = Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        if ($item -and $item.LinkType) {
            $backup = Resolve-BundlePath $backupRoot $relative
            [void][IO.Directory]::CreateDirectory((Split-Path -Parent $backup))
            Move-Item -LiteralPath $path -Destination $backup
        }
    }
}
$updated = 0
foreach ($file in $manifest.files) {
    $source = Resolve-BundlePath $packageRoot $file.path
    $destination = Resolve-BundlePath $installRoot $file.target
    if ($ApplyAfterExit) {
        while (@(Get-TargetProcess).Count) { Start-Sleep -Seconds 1 }
    }
    if (Test-Path -LiteralPath $destination -PathType Leaf) {
        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -eq $file.sha256) { continue }
        $backup = Resolve-BundlePath $backupRoot $file.target
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $backup))
        Copy-Item -LiteralPath $destination -Destination $backup -Force
    }
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
    for ($attempt = 0; ; $attempt++) {
        try {
            Copy-Item -LiteralPath $source -Destination $destination -Force
            break
        } catch {
            if ($attempt -ge 29) { throw }
            Start-Sleep -Seconds 1
        }
    }
    if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $file.sha256) {
        throw "Installed file verification failed: $($file.target)"
    }
    $updated++
}
[void][IO.Directory]::CreateDirectory((Join-Path $installRoot 'portable-data'))
$extensionsDirectory = Join-Path $installRoot 'portable-data/extensions/installed'
if ($updated -and !@(Get-TargetProcess).Count) {
    [IO.Directory]::SetLastWriteTimeUtc($extensionsDirectory, [DateTime]::UtcNow)
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $installRoot 'installed-bundle.json') -Encoding utf8
if ($ApplyAfterExit) {
    @{status='completed';commit=$manifest.commit;automatic_restart=$false} |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $installRoot 'pending-update.json') -Encoding utf8
}
Write-Host "Installed commit $($manifest.commit): $updated files updated; $($manifest.plugins.Count) plugins verified."
if (Test-Path -LiteralPath $backupRoot) { Write-Host "Replaced files backed up to: $backupRoot" }
Write-Host 'Existing settings and personal data were preserved. Zed was not restarted.'
