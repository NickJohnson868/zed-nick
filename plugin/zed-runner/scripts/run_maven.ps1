#requires -Version 5.1
param(
    [Parameter(Mandatory=$true)][string]$Root,
    [Parameter(Mandatory=$true)][string]$Pom,
    [Parameter(Mandatory=$true)][string]$Goals,
    [string]$ReactorPom,
    [string]$Profiles,
    [switch]$SkipTests,
    [switch]$Offline,
    [switch]$AlsoMake
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/platform.ps1"
try {
    $Root = (Resolve-Path -LiteralPath $Root -ErrorAction Stop).ProviderPath
    $Pom = (Resolve-Path -LiteralPath $Pom -ErrorAction Stop).ProviderPath
    if (!$ReactorPom) { $ReactorPom = $Pom }
    $ReactorPom = (Resolve-Path -LiteralPath $ReactorPom -ErrorAction Stop).ProviderPath
    foreach ($path in @($Pom,$ReactorPom)) {
        if (!(Test-RunnerPathWithin $path $Root) -or [IO.Path]::GetFileName($path) -ne 'pom.xml') { throw '只能执行当前工作区内的 pom.xml。' }
    }
    $goalArguments = @($Goals.Split(',') | Where-Object { $_ })
    if (!$goalArguments.Count) { throw '请选择 Maven 目标。' }
    foreach ($goal in $goalArguments) {
        if ($goal -notmatch '^[A-Za-z0-9_][A-Za-z0-9_.:-]*$' -and $goal -notmatch '^-Dplugin=[A-Za-z0-9_.:-]+$' -and $goal -ne '-Ddetail') { throw "无效 Maven 目标：$goal" }
    }
    $configuration = $null
    $settingsFile = Join-Path $Root '.zed/run-environments.json'
    if (Test-Path -LiteralPath $settingsFile -PathType Leaf) { $configuration = Get-Content -LiteralPath $settingsFile -Raw -Encoding UTF8 | ConvertFrom-Json }
    $settings = $configuration.environments.java
    if ($settings.env) {
        foreach ($variable in $settings.env.PSObject.Properties) { [Environment]::SetEnvironmentVariable($variable.Name,[string]$variable.Value,'Process') }
    }
    $defaults = & "$PSScriptRoot/../config-template.ps1" | ConvertFrom-Json
    $javaHome = [string]$settings.runtime
    if (!$javaHome) { $javaHome = [string]$settings.env.JAVA_HOME }
    if (!$javaHome) { $javaHome = [string]$defaults.environments.java.runtime }
    if ($javaHome) {
        if (!(Test-RunnerAbsolutePath $javaHome) -or !(Test-Path -LiteralPath (Join-Path $javaHome $runnerJavaRelativePath) -PathType Leaf)) { throw 'java.runtime 必须是当前平台的 JDK 绝对目录。' }
        $env:JAVA_HOME = $javaHome
        $env:PATH = (Join-Path $javaHome 'bin') + [IO.Path]::PathSeparator + $env:PATH
    }
    $buildTool = [string]$settings.build_tool
    $arguments = @()
    $reactorDirectory = Split-Path $ReactorPom -Parent
    if (!$buildTool) {
        $wrapper = Join-Path $reactorDirectory $(if ($IsWindows) { 'mvnw.cmd' } else { 'mvnw' })
        if (Test-Path -LiteralPath $wrapper -PathType Leaf) {
            if ($IsWindows) { $buildTool = $wrapper } else { $buildTool = '/bin/sh'; $arguments += $wrapper }
        } else { $buildTool = [string]$defaults.environments.java.build_tool }
    }
    if ($buildTool -and (Test-Path -LiteralPath $buildTool -PathType Container)) {
        $candidates = if ($IsWindows) { @('bin/mvnd.exe','bin/mvn.cmd','mvnd.exe','mvn.cmd') } else { @('bin/mvnd','bin/mvn','mvnd','mvn') }
        $buildTool = @($candidates | ForEach-Object { Join-Path $buildTool $_ } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1)[0]
    }
    if (!$buildTool -or !(Test-RunnerAbsolutePath $buildTool) -or !(Test-Path -LiteralPath $buildTool -PathType Leaf)) { throw '未找到 Maven；请在 .zed/run-environments.json 的 java.build_tool 中配置 Maven/MVND，或提供 mvnw。' }
    Set-Location -LiteralPath $reactorDirectory
    if ($AlsoMake -and !$Pom.Equals($ReactorPom,$runnerPathComparison)) {
        $relative = (Get-RunnerRelativePath $reactorDirectory (Split-Path $Pom -Parent))
        if ((Test-RunnerAbsolutePath $relative)) { throw '所选模块不在当前 Reactor 内。' }
        $arguments += @('-f',$ReactorPom,'-pl',$relative,'-am')
    } else { $arguments += @('-f',$Pom) }
    $arguments += '-B'
    if ($SkipTests) { $arguments += '-DskipTests' }
    if ($Offline) { $arguments += '-o' }
    if ($Profiles) { $arguments += @('-P',$Profiles) }
    $arguments += $goalArguments
    Write-Host "Maven · $([IO.Path]::GetFileName((Split-Path $Pom -Parent))) · $Goals" -ForegroundColor Cyan
    & $buildTool @arguments
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
