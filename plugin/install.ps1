#requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ExtensionsDirectory,
    [string]$ExtensionBuilder,
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '../.local-plugin-build'),
    [switch]$SkipBuild,
    [switch]$SkipCommunity
)
$ErrorActionPreference = 'Stop'
# Build-machine convenience wrapper; this file is never shipped as a plugin runtime.
$python = Get-Command python.exe, python3.exe, python, python3 -ErrorAction SilentlyContinue | Select-Object -First 1
if (!$python) { throw '构建机需要 Python 3.12+；已打包插件的使用者不需要安装 Python。' }
$arguments = @((Join-Path $PSScriptRoot 'install.py'), '--extensions-directory', $ExtensionsDirectory, '--build-directory', $BuildDirectory)
if ($ExtensionBuilder) { $arguments += @('--extension-builder', $ExtensionBuilder) }
if ($SkipBuild) { $arguments += '--skip-build' }
if ($SkipCommunity) { $arguments += '--skip-community' }
& $python.Source @arguments
if ($LASTEXITCODE -ne 0) { throw "插件构建失败，退出码：$LASTEXITCODE" }
