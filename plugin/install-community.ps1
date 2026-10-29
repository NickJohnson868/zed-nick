#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ExtensionsDirectory,
    [string]$CacheDirectory = (Join-Path $PSScriptRoot '../.local-plugin-build/community')
)
$ErrorActionPreference = 'Stop'
$manifest = Get-Content -LiteralPath "$PSScriptRoot/community.json" -Raw -Encoding UTF8 | ConvertFrom-Json
New-Item -ItemType Directory -Path $CacheDirectory -Force | Out-Null
foreach ($extension in $manifest.extensions) {
    if ($extension.id -notmatch '^[a-z0-9-]+$' -or $extension.version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid community extension identity' }
    $expectedUrl = "https://api.zed.dev/extensions/$($extension.id)/$($extension.version)/download"
    if ($extension.download -ne $expectedUrl) { throw 'Expected an official Zed extension download' }
    $archive = Join-Path $CacheDirectory "$($extension.id)-$($extension.version).tar.gz"
    if (!(Test-Path -LiteralPath $archive)) { Invoke-WebRequest -Uri $extension.download -OutFile $archive -UseBasicParsing }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $extension.sha256) { throw "Checksum mismatch: $archive" }
    $entries = @(& tar -tzf $archive)
    if ($LASTEXITCODE) { throw "Invalid extension archive: $archive" }
    foreach ($entry in $entries) {
        if ($entry -match '(^[/\\]|^[A-Za-z]:|(^|[/\\])\.\.([/\\]|$))') { throw "Invalid archive path: $entry" }
    }
    $stage = Join-Path $CacheDirectory ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    & tar -xzf $archive -C $stage
    if ($LASTEXITCODE) { throw "Cannot extract $archive" }
    $metadata = Get-Content -LiteralPath (Join-Path $stage 'extension.toml') -Raw -Encoding UTF8
    if ($metadata -notmatch ('(?m)^id\s*=\s*"' + [regex]::Escape($extension.id) + '"') -or
        $metadata -notmatch ('(?m)^version\s*=\s*"' + [regex]::Escape($extension.version) + '"')) { throw "Unexpected extension manifest: $archive" }
    $destination = Join-Path $ExtensionsDirectory "installed/$($extension.id)"
    if ((Test-Path -LiteralPath $destination) -and (Get-Item -LiteralPath $destination -Force).LinkType) { throw "Preserving linked extension: $destination" }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $stage -Force) { Copy-Item -LiteralPath $item.FullName -Destination $destination -Recurse -Force }
    Write-Host "Installed community extension: $($extension.id) $($extension.version)"
}
