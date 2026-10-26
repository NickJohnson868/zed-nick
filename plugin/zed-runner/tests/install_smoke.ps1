#requires -Version 7.0
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ('zed-plugin-install-' + [guid]::NewGuid().ToString('N'))
$build = Join-Path $root 'build'
$extensions = Join-Path $root 'extensions'
$hostTriple = ((& rustc -vV | Select-String '^host: ').Line -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0 -or !$hostTriple) { throw 'Rust host unavailable' }
$suffix = if ($IsWindows) { '.exe' } else { '' }
New-Item -ItemType Directory -Path "$build/java","$build/java-stage","$build/target/$hostTriple/release","$build/target/wasm32-wasip2/release" -Force | Out-Null
Set-Content -LiteralPath "$build/java-stage/extension.toml" -Value 'id = "java"'
& tar -czf "$build/java/archive.tar.gz" -C "$build/java-stage" .
if ($LASTEXITCODE -ne 0) { throw 'Fixture archive failed' }
Set-Content -LiteralPath "$build/target/wasm32-wasip2/release/zed_mybatis_jump.wasm" -Value 'fixture-wasm'
$installer = Join-Path $PSScriptRoot '../../install.ps1'
$rejected = $false
try { & $installer -ExtensionsDirectory $extensions -BuildDirectory $build -SkipBuild } catch { $rejected = $true }
if (!$rejected -or (Test-Path -LiteralPath $extensions)) { throw 'Missing native artifacts must fail before installation' }
foreach ($name in @('java-lsp-proxy','gradle-lsp-bridge','mybatis-jump-lsp')) {
    Set-Content -LiteralPath "$build/target/$hostTriple/release/$name$suffix" -Value "fixture-$hostTriple-$name"
}
& $installer -ExtensionsDirectory $extensions -BuildDirectory $build -SkipBuild
foreach ($id in @('java','mybatis-jump','zed-runner','idea-islands-2026')) {
    if (!(Test-Path -LiteralPath "$extensions/installed/$id/extension.toml")) { throw "Missing installed extension: $id" }
}
foreach ($entry in @(@('java','java-lsp-proxy'),@('java','gradle-lsp-bridge'),@('mybatis-jump','mybatis-jump-lsp'))) {
    $path = "$extensions/work/$($entry[0])/bin/$($entry[1])$suffix"
    if ((Get-Content -LiteralPath $path -Raw).Trim() -ne "fixture-$hostTriple-$($entry[1])") { throw "Wrong native binary: $path" }
}
if (!(Test-Path -LiteralPath "$extensions/installed/zed-runner/scripts/platform.ps1")) { throw 'Runner platform helper omitted' }
Write-Host 'Passed: installer artifact guard, all plugin payloads and current-host native filenames.'

exit 0
