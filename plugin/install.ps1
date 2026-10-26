#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ExtensionsDirectory,
    [string]$ExtensionBuilder,
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '../.local-plugin-build'),
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$ExtensionsDirectory = [IO.Path]::GetFullPath($ExtensionsDirectory)
$buildDirectory = [IO.Path]::GetFullPath($BuildDirectory)
$hostTriple = ((& rustc -vV | Select-String '^host: ').Line -replace '^host: ', '').Trim()
if ($LASTEXITCODE -ne 0 -or !$hostTriple) { throw '无法确定 Rust 本机编译目标' }
$executableSuffix = if ($IsWindows) { '.exe' } else { '' }
$previousTarget = $env:CARGO_TARGET_DIR
$originalLocation = Get-Location
function Invoke-Build([string[]]$Arguments) {
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "构建失败：cargo $($Arguments -join ' ')" }
}
function Copy-Runtime([string]$Source, [string]$Destination) {
    if (!(Test-Path -LiteralPath $Source)) { throw "缺少插件产物：$Source" }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
        if ($item.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) { throw "插件产物不能包含链接：$($item.FullName)" }
        Copy-Item -LiteralPath $item.FullName -Destination $Destination -Recurse -Force
    }
}
try {
    Set-Location -LiteralPath $repository
    # Separate these outputs from the application's Cargo feature combinations.
    $env:CARGO_TARGET_DIR = Join-Path $buildDirectory 'target'
    if (!$ExtensionBuilder) { $ExtensionBuilder = Join-Path $env:CARGO_TARGET_DIR "$hostTriple/release/zed-extension$executableSuffix" }
    if (!$SkipBuild) {
        & rustup target add wasm32-wasip2
        if ($LASTEXITCODE -ne 0) { throw '无法安装 WASM 编译目标' }
        if (!(Test-Path -LiteralPath $ExtensionBuilder)) {
            Invoke-Build @('build','--locked','--release','--target',$hostTriple,'-p','extension_cli','--bin','zed-extension')
        }
        Invoke-Build @('build','--locked','--release','--target',$hostTriple,'--manifest-path',"$PSScriptRoot/zed-java-fork/Cargo.toml",'-p','java-lsp-proxy','-p','gradle-lsp-bridge')
        Invoke-Build @('build','--locked','--release','--target',$hostTriple,'--manifest-path',"$PSScriptRoot/zed-mybatis-jump/Cargo.toml",'-p','mybatis-jump-lsp')
        Invoke-Build @('build','--locked','--release','--target','wasm32-wasip2','--manifest-path',"$PSScriptRoot/zed-mybatis-jump/Cargo.toml",'-p','zed-mybatis-jump')
        # Zed's builder compiles the Java grammars as well as extension.wasm.
        & $ExtensionBuilder --source-dir "$PSScriptRoot/zed-java-fork" --output-dir "$buildDirectory/java" --scratch-dir "$buildDirectory/scratch"
        if ($LASTEXITCODE -ne 0) { throw 'Java 扩展及语法包构建失败' }
    }
    foreach ($relative in @('java/archive.tar.gz',"target/wasm32-wasip2/release/zed_mybatis_jump.wasm","target/$hostTriple/release/java-lsp-proxy$executableSuffix","target/$hostTriple/release/gradle-lsp-bridge$executableSuffix","target/$hostTriple/release/mybatis-jump-lsp$executableSuffix")) {
        if (!(Test-Path -LiteralPath (Join-Path $buildDirectory $relative) -PathType Leaf)) { throw "缺少当前平台产物：$relative" }
    }
    foreach ($id in @('java','mybatis-jump','zed-runner','idea-islands-2026')) {
        foreach ($relative in @("installed/$id","work/$id","work/$id/bin")) {
            $destination = Join-Path $ExtensionsDirectory $relative
            if ((Test-Path -LiteralPath $destination) -and (Get-Item -LiteralPath $destination -Force).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) { throw "保留现有开发链接，请更新对应源目录：$destination" }
        }
    }
    $stage = Join-Path $buildDirectory ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path (Join-Path $stage 'java') -Force | Out-Null
    & tar -xzf "$buildDirectory/java/archive.tar.gz" -C (Join-Path $stage 'java')
    if ($LASTEXITCODE -ne 0) { throw 'Java 扩展包解压失败' }
    Copy-Runtime "$PSScriptRoot/zed-idea-islands-2026" (Join-Path $stage 'idea-islands-2026')
    $runner = Join-Path $stage 'zed-runner'
    New-Item -ItemType Directory -Path $runner -Force | Out-Null
    foreach ($name in @('extension.toml','run-provider.ps1','config-template.ps1','run-environments.template.json','scripts')) {
        Copy-Item -LiteralPath (Join-Path "$PSScriptRoot/zed-runner" $name) -Destination $runner -Recurse -Force
    }
    $mybatis = Join-Path $stage 'mybatis-jump'
    New-Item -ItemType Directory -Path $mybatis -Force | Out-Null
    Copy-Item -LiteralPath "$PSScriptRoot/zed-mybatis-jump/extension.toml" -Destination $mybatis
    Copy-Item -LiteralPath "$env:CARGO_TARGET_DIR/wasm32-wasip2/release/zed_mybatis_jump.wasm" -Destination (Join-Path $mybatis 'extension.wasm')
    foreach ($id in @('java','mybatis-jump','zed-runner','idea-islands-2026')) {
        $destination = Join-Path $ExtensionsDirectory "installed/$id"
        if ((Test-Path -LiteralPath $destination) -and (Get-Item -LiteralPath $destination -Force).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
            throw "现有插件是开发链接，请保留链接并更新源目录：$destination"
        }
        Copy-Runtime (Join-Path $stage $id) $destination
    }
    foreach ($entry in @(@('java','java-lsp-proxy'),@('java','gradle-lsp-bridge'),@('mybatis-jump','mybatis-jump-lsp'))) {
        $binary = Join-Path $env:CARGO_TARGET_DIR "$hostTriple/release/$($entry[1])$executableSuffix"
        $destination = Join-Path $ExtensionsDirectory "work/$($entry[0])/bin"
        New-Item -ItemType Directory -Path $destination -Force | Out-Null
        Copy-Item -LiteralPath $binary -Destination $destination -Force
        if (!$IsWindows) {
            & chmod +x (Join-Path $destination "$($entry[1])$executableSuffix")
            if ($LASTEXITCODE -ne 0) { throw '无法设置原生插件执行权限' }
        }
    }
    Write-Host "已安装当前平台插件（$hostTriple）：$ExtensionsDirectory。下次启动 ZedNick 加载；未重启。"
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Set-Location -LiteralPath $originalLocation
}
