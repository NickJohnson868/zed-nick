#requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateRange(1, 128)][int]$Jobs = 8,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '.local-portable-build'),
    [string]$TargetDirectory = (Join-Path $env:USERPROFILE 'Documents\tools\rust\targets\zed'),
    [string]$SourceDataDirectory = (Join-Path $env:LOCALAPPDATA 'Zed'),
    [string]$SourceConfigDirectory = (Join-Path $env:APPDATA 'Zed'),
    [switch]$ImportProfile,
    [switch]$SkipProfileImport,
    [switch]$SkipBuild,
    [switch]$SkipPluginBuild,
    [string]$PluginBuildDirectory = (Join-Path $PSScriptRoot '.local-plugin-build')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

function Copy-Directory([string]$Source, [string]$Destination) {
    if (!(Test-Path -LiteralPath $Source -PathType Container)) { return }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    & robocopy.exe $Source $Destination /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /XJ /XF 'zed-crash-handler-*' '*.sock' '*.socket' /NFL /NDL /NJH /NJS /NP
    if ($LASTEXITCODE -ge 8) { throw "复制失败（robocopy $LASTEXITCODE）：$Source" }
}

function Assert-NoZedProcess([string]$Directory, [string]$Reason) {
    $prefix = [IO.Path]::GetFullPath($Directory).TrimEnd('\') + '\'
    $running = @(Get-Process -Name Zed -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    })
    if ($running.Count) { throw "$Reason，请先关闭：$($running.Path -join ', ')" }
}

$savedEnvironment = @{}
$environmentNames = @('CARGO_HOME', 'RUSTUP_HOME', 'CARGO_TARGET_DIR', 'CARGO_BUILD_JOBS', 'CARGO_PROFILE_RELEASE_FAST_DEBUG', 'ZED_RELEASE_CHANNEL', 'ZED_UPDATE_EXPLANATION', 'PATH')
foreach ($name in $environmentNames) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$previousLocation = Get-Location
$buildProcess = Get-Process -Id $PID
$previousPriority = $buildProcess.PriorityClass
try {
    Set-Location -LiteralPath $PSScriptRoot
    $outputPath = [IO.Path]::GetFullPath($OutputDirectory)
    $targetPath = [IO.Path]::GetFullPath($TargetDirectory)
    if ($outputPath -eq [IO.Path]::GetFullPath($PSScriptRoot) -or $outputPath -eq $targetPath) {
        throw '输出目录必须独立于源码根目录和 Cargo target 目录。'
    }
    Assert-NoZedProcess $outputPath '不能覆盖正在运行的绿色版'
    $profile = Join-Path $outputPath 'portable-data'
    if ($ImportProfile -and $SkipProfileImport) { throw '-ImportProfile 与 -SkipProfileImport 不能同时使用。' }
    if (!$ImportProfile -and (Test-Path -LiteralPath $profile) -and
        ((Get-Item -LiteralPath $profile -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "通用打包不能复用链接的 portable-data：$profile。请指定新的空输出目录。"
    }
    if (!$ImportProfile -and (Test-Path -LiteralPath $profile -PathType Container) -and
        (Get-ChildItem -LiteralPath $profile -Force | Where-Object {
            $_.Name -ne 'extensions' -or !(Test-Path -LiteralPath (Join-Path $outputPath 'bundle-manifest.json'))
        } | Select-Object -First 1)) {
        throw "通用打包不能复用已有数据的 portable-data：$profile。请指定一个新的空输出目录；原有个人数据不会被删除。"
    }
    $importProfile = $ImportProfile -and !(Test-Path -LiteralPath $profile)
    if ($importProfile) {
        if (@(Get-Process -Name Zed -ErrorAction SilentlyContinue).Count) {
            throw '首次迁移配置和数据库前，请关闭所有 Zed 窗口后重试，避免复制使用中的数据库。不导入配置时省略 -ImportProfile。'
        }
        $sourceDataPath = [IO.Path]::GetFullPath($SourceDataDirectory).TrimEnd('\') + '\'
        if (($profile.TrimEnd('\') + '\').StartsWith($sourceDataPath, [StringComparison]::OrdinalIgnoreCase)) {
            throw '绿色版输出目录不能放在原 Zed 数据目录内。'
        }
    }

    if (!$SkipBuild) {
        $buildProcess.PriorityClass = [Diagnostics.ProcessPriorityClass]::BelowNormal
        $rustRoot = Join-Path $env:USERPROFILE 'Documents\tools\rust'
        if (Test-Path -LiteralPath (Join-Path $rustRoot 'cargo\bin\cargo.exe')) {
            $env:CARGO_HOME = Join-Path $rustRoot 'cargo'
            $env:RUSTUP_HOME = Join-Path $rustRoot 'rustup'
            $env:PATH = (Join-Path $env:CARGO_HOME 'bin') + ';' + $env:PATH
        }
        $cmake = 'C:\Program Files\Microsoft Visual Studio\18\Professional\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
        if (Test-Path -LiteralPath $cmake) { $env:PATH = $cmake + ';' + $env:PATH }
        if (!(Get-Command cargo -ErrorAction SilentlyContinue)) { throw '未找到 Rust cargo，请先安装 Rust。' }
        $env:CARGO_TARGET_DIR = $targetPath
        $env:CARGO_BUILD_JOBS = [string]$Jobs
        $env:CARGO_PROFILE_RELEASE_FAST_DEBUG = '0'
        $env:ZED_RELEASE_CHANNEL = 'stable'
        $env:ZED_UPDATE_EXPLANATION = '个人绿色版，请通过 zed-nick 源码更新。'
        Write-Host "使用 $Jobs 个并行任务编译，保留增量缓存，不执行 clean。" -ForegroundColor Cyan
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] [1/4] 检查 Rust 格式（成功时无额外输出）" -ForegroundColor Cyan
        & cargo fmt --all -- --check
        if ($LASTEXITCODE) { throw 'Rust 格式检查失败。' }
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] [2/4] 编译 Zed GUI；单个大型 crate 编译期间可能暂时没有新日志" -ForegroundColor Cyan
        & cargo build --locked -p zed --bin zed --profile release-fast --jobs $Jobs
        if ($LASTEXITCODE) { throw 'Zed 编译失败，未组装绿色版。' }
        # Keep CLI and GUI builds separate to avoid unnecessary feature unification.
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] [3/4] 编译 Zed CLI" -ForegroundColor Cyan
        & cargo build --locked -p cli --bin cli --profile release-fast --jobs $Jobs
        if ($LASTEXITCODE) { throw 'Zed CLI 编译失败，未组装绿色版。' }
    }

    Write-Host "[$(Get-Date -Format 'HH:mm:ss')] [4/4] 组装绿色版：$outputPath" -ForegroundColor Cyan
    $build = Join-Path $targetPath 'release-fast'
    $required = @('zed.exe', 'cli.exe', 'conpty.dll')
    foreach ($relative in $required) {
        if (!(Test-Path -LiteralPath (Join-Path $build $relative) -PathType Leaf)) { throw "缺少构建产物：$relative" }
    }
    $agsCandidates = @(
        (Join-Path $build 'amd_ags_x64.dll'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Zed\amd_ags_x64.dll'),
        (Join-Path $env:USERPROFILE 'Documents\tools\zed-personal\amd_ags_x64.dll')
    )
    $ags = $agsCandidates | Where-Object {
        (Test-Path -LiteralPath $_ -PathType Leaf) -and
        (Get-Item -LiteralPath $_).VersionInfo.FileVersion -eq '6.3.0.0'
    } | Select-Object -First 1
    if (!$ags) {
        # Match the SDK pinned by script/bundle-windows.ps1.
        $agsCache = Join-Path $targetPath 'portable-deps\ags-6.3.0'
        $ags = Join-Path $agsCache 'AGS_SDK-6.3.0\ags_lib\lib\amd_ags_x64.dll'
        if (!(Test-Path -LiteralPath $ags -PathType Leaf)) {
            New-Item -ItemType Directory -Path $agsCache -Force | Out-Null
            $agsArchive = Join-Path $agsCache 'AGS_SDK_v6.3.0.zip'
            Write-Host '获取 AMD AGS SDK 6.3.0 运行依赖…' -ForegroundColor Cyan
            Invoke-WebRequest -Uri 'https://codeload.github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/zip/refs/tags/v6.3.0' -OutFile $agsArchive
            Expand-Archive -LiteralPath $agsArchive -DestinationPath $agsCache -Force
        }
        if (!(Test-Path -LiteralPath $ags -PathType Leaf)) { throw '未找到 AMD AGS 6.3.0 DLL。' }
    }
    $consoles = @{}
    foreach ($architecture in @('x64', 'arm64')) {
        $consoleCandidates = @((Join-Path $build "$architecture\OpenConsole.exe"))
        if ($architecture -eq 'x64') { $consoleCandidates += Join-Path $build 'OpenConsole.exe' }
        foreach ($buildOutput in @(Get-ChildItem -LiteralPath (Join-Path $build 'build') -Directory -Filter 'zed-*')) {
            $consoleCandidates += Join-Path $buildOutput.FullName "out\conpty\build\native\runtimes\$architecture\OpenConsole.exe"
        }
        $console = $consoleCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
        if ($console) { $consoles[$architecture] = $console }
    }
    if (!$consoles.ContainsKey('x64')) { throw '缺少构建产物：x64 OpenConsole.exe' }
    New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $outputPath 'bin') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $build 'zed.exe') -Destination (Join-Path $outputPath 'Zed.exe') -Force
    Copy-Item -LiteralPath (Join-Path $build 'cli.exe') -Destination (Join-Path $outputPath 'bin\zed.exe') -Force
    Copy-Item -LiteralPath (Join-Path $build 'conpty.dll') -Destination (Join-Path $outputPath 'conpty.dll') -Force
    Copy-Item -LiteralPath $ags -Destination (Join-Path $outputPath 'amd_ags_x64.dll') -Force
    foreach ($architecture in @('x64', 'arm64')) {
        if ($consoles.ContainsKey($architecture)) {
            New-Item -ItemType Directory -Path (Join-Path $outputPath $architecture) -Force | Out-Null
            Copy-Item -LiteralPath $consoles[$architecture] -Destination (Join-Path $outputPath "$architecture\OpenConsole.exe") -Force
        }
    }
    if ($importProfile) {
        # Stage a first import so a failed copy is not mistaken for a complete profile.
        $staging = Join-Path $outputPath ('profile-import-' + [Guid]::NewGuid().ToString('N'))
        Copy-Directory $SourceDataDirectory $staging
        $installedExtensions = Join-Path $SourceDataDirectory 'extensions\installed'
        if (Test-Path -LiteralPath $installedExtensions -PathType Container) {
            foreach ($extension in @(Get-ChildItem -LiteralPath $installedExtensions -Directory -Force | Where-Object { $_.LinkType })) {
                $target = $extension.ResolveLinkTarget($true)
                if (!$target -or !(Test-Path -LiteralPath $target.FullName -PathType Container)) {
                    throw "开发扩展链接不可用：$($extension.Name)"
                }
                $destination = Join-Path $staging "extensions\installed\$($extension.Name)"
                if (!(Test-Path -LiteralPath $destination)) {
                    New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
                    New-Item -ItemType Junction -Path $destination -Target $target.FullName | Out-Null
                }
            }
        }
        Copy-Directory $SourceConfigDirectory (Join-Path $staging 'config')
        New-Item -ItemType Directory -Path $staging -Force | Out-Null
        $outputPrefix = $outputPath.TrimEnd('\') + '\'
        if (!([IO.Path]::GetFullPath($staging).StartsWith($outputPrefix, [StringComparison]::OrdinalIgnoreCase)) -or
            !([IO.Path]::GetFullPath($profile).StartsWith($outputPrefix, [StringComparison]::OrdinalIgnoreCase))) {
            throw '配置迁移路径超出绿色版目录。'
        }
        [IO.Directory]::Move($staging, $profile)
    } else {
        New-Item -ItemType Directory -Path $profile -Force | Out-Null
    }
    & "$PSScriptRoot/plugin/install.ps1" -ExtensionsDirectory (Join-Path $profile 'extensions') -BuildDirectory $PluginBuildDirectory -SkipBuild:$SkipPluginBuild
    @'
直接双击本目录的 Zed.exe，不需要 CMD、PowerShell 启动器或安装。
portable-data 内预装插件，首次启动后将配置和数据保存在程序旁边。
通用 ZIP 自带 plugin 目录定义的插件，不包含个人设置、登录信息、数据库、日志或语言服务下载缓存。
首次启动会自动创建所需内容；更新程序时保留自己机器上的 portable-data。
项目里的 .zed/run-environments.json 配置各语言运行环境；下拉框选择入口。
'@ | Set-Content -LiteralPath (Join-Path $outputPath '使用说明.txt') -Encoding utf8
    Write-Host "绿色版已生成：$(Join-Path $outputPath 'Zed.exe')" -ForegroundColor Green

    $archivePath = Join-Path $PSScriptRoot 'ZedNick-Portable.zip'
    $temporaryArchive = Join-Path $PSScriptRoot ('.local-portable-zip-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    Copy-Item -LiteralPath "$PSScriptRoot/script/update-portable.ps1" -Destination (Join-Path $outputPath 'Update-Zed.ps1') -Force
    $files = @('Zed.exe', 'bin\zed.exe', 'conpty.dll', 'amd_ags_x64.dll', 'x64\OpenConsole.exe', '使用说明.txt', 'Update-Zed.ps1')
    if (Test-Path -LiteralPath (Join-Path $outputPath 'arm64\OpenConsole.exe') -PathType Leaf) {
        $files += 'arm64\OpenConsole.exe'
    }
    foreach ($directory in @('installed/java','installed/mybatis-jump','installed/zed-runner','installed/html','installed/sql','installed/xml','work/java/bin','work/mybatis-jump/bin')) {
        $directoryPath = Join-Path $profile "extensions/$directory"
        foreach ($file in Get-ChildItem -LiteralPath $directoryPath -File -Recurse) {
            $files += [IO.Path]::GetRelativePath($outputPath, $file.FullName)
        }
    }
    $plugins = foreach ($id in @('java','mybatis-jump','zed-runner','html','sql','xml')) {
        $metadata = Get-Content -LiteralPath (Join-Path $profile "extensions/installed/$id/extension.toml") -Raw
        if ($metadata -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw "插件缺少版本：$id" }
        @{ id=$id; version=$Matches[1] }
    }
    $bundleFiles = foreach ($relative in $files) {
        $source = Join-Path $outputPath $relative
        @{ path=$relative.Replace('\','/'); target=$relative.Replace('\','/'); sha256=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant(); size=(Get-Item -LiteralPath $source).Length }
    }
    @{ commit=(& git rev-parse HEAD).Trim(); built_at=[DateTimeOffset]::Now.ToString('o'); platform='windows-x64'; local_uncommitted_ui_changes=([bool](& git status --porcelain)); plugins=@($plugins); files=@($bundleFiles) } |
        ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputPath 'bundle-manifest.json') -Encoding utf8
    $files += 'bundle-manifest.json'
    try {
        Add-Type -AssemblyName System.IO.Compression
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [IO.Compression.ZipFile]::Open($temporaryArchive, [IO.Compression.ZipArchiveMode]::Create)
        try {
            [void]$archive.CreateEntry('ZedNick-Portable/')
            [void]$archive.CreateEntry('ZedNick-Portable/portable-data/')
            foreach ($relative in $files) {
                $source = Join-Path $outputPath $relative
                if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "绿色版缺少文件：$relative" }
                $name = 'ZedNick-Portable/' + $relative.Replace('\', '/')
                [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $archive, $source, $name, [IO.Compression.CompressionLevel]::Optimal
                )
            }
        } finally { $archive.Dispose() }
        [IO.File]::Move($temporaryArchive, $archivePath, $true)
        $check = [IO.Compression.ZipFile]::OpenRead($archivePath)
        try {
            $dataEntries = @($check.Entries | Where-Object { $_.FullName.StartsWith('ZedNick-Portable/portable-data/', [StringComparison]::Ordinal) })
            if (@($dataEntries | Where-Object { $_.FullName -ne 'ZedNick-Portable/portable-data/' -and !$_.FullName.StartsWith('ZedNick-Portable/portable-data/extensions/', [StringComparison]::Ordinal) }).Count) {
                throw '通用 ZIP 不得包含插件以外的个人数据。'
            }
        } finally { $check.Dispose() }
    } finally {
        if (Test-Path -LiteralPath $temporaryArchive -PathType Leaf) {
            Remove-Item -LiteralPath $temporaryArchive -Force
        }
    }
    Write-Host "绿色 ZIP 已生成：$archivePath" -ForegroundColor Green
} finally {
    $buildProcess.PriorityClass = $previousPriority
    Set-Location -LiteralPath $previousLocation
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
}
