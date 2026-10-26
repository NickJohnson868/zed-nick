#requires -Version 7.0
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/../scripts/platform.ps1"
. "$PSScriptRoot/../scripts/workspace_files.ps1"
$root = Join-Path ([IO.Path]::GetTempPath()) ('zed-scan-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path "$root/.zed","$root/app/tools","$root/other","$root/app/src" -Force | Out-Null
Set-Content "$root/app/src/main.py" 'pass'
Set-Content "$root/app/tools/skipped.py" 'pass'
Set-Content "$root/other/skipped.py" 'pass'
Set-Content "$root/.zed/runner-ignore.json" '{"scan_directories":["app"],"ignore_directories":["tools"]}'
$files = @(Get-WorkspaceFiles $root)
if ($files.Count -ne 1 -or $files[0] -ne 'app/src/main.py') { throw "Whitelist/ignore regression: $files" }
if (Test-RunnerPathWithin ($root + '-sibling/main.py') $root) { throw 'Sibling path escaped boundary check' }
& git -C $root init -q
if ($LASTEXITCODE -ne 0) { throw 'Test git init failed' }
Set-Content "$root/.gitignore" 'app/src/'
$files = @(Get-WorkspaceFiles $root)
if ($files.Count) { throw "Git ignores not applied: $files" }
Write-Host 'Passed: native path separators, whitelist, nested excludes, Git ignore, sibling boundary.'
