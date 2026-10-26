param([switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$argumentFile = $null
$mutex = $null
$locked = $false
. "$PSScriptRoot/platform.ps1"
. "$PSScriptRoot/workspace_files.ps1"

function Get-InputStamp([string]$Root, [string[]]$Modules, [string]$Identity) {
    $hash = [Security.Cryptography.IncrementalHash]::CreateHash([Security.Cryptography.HashAlgorithmName]::SHA256)
    $files = [Collections.Generic.List[string]]::new()
    try {
        foreach ($relative in @(Get-WorkspaceFiles $Root -StrictFallback)) {
            $path = Join-Path $Root $relative
            if (!(Test-Path -LiteralPath $path -PathType Leaf)) { continue }
            $item = [IO.FileInfo]::new($path)
            $source = $relative -match '(^|/)src/' -or $item.Extension -in @('.java','.kt','.scala','.groovy','.proto')
            $inScope = !$Modules.Count
            if ($source -and !$inScope) {
                foreach ($module in $Modules) {
                    if ((Test-RunnerPathWithin $item.FullName $module)) { $inScope=$true; break }
                }
            }
            if ($item.Name -eq 'pom.xml' -or ($inScope -and $source) -or $relative.StartsWith('.mvn/') -or $item.Name -in @('mvnw','mvnw.cmd')) { $files.Add($item.FullName) }
        }
        foreach ($path in @((Join-Path $Root 'pom.xml'),(Join-Path $Root '.zed/run-config.json'),(Join-Path $Root '.zed/run-environments.json'),(Join-Path $Root '.zed/runner-ignore.json'),(Join-Path $runnerHome '.m2/settings.xml'),(Join-Path $runnerHome '.m2/toolchains.xml'))) {
            if (Test-Path -LiteralPath $path -PathType Leaf) { $files.Add($path) }
        }
        $files.Sort([StringComparer]::Ordinal)
        $hash.AppendData([Text.Encoding]::UTF8.GetBytes($Identity))
        foreach ($path in $files) {
            $hash.AppendData([Text.Encoding]::UTF8.GetBytes($path+"`0"))
            $stream = [IO.File]::OpenRead($path)
            try { $hash.AppendData([Security.Cryptography.SHA256]::HashData($stream)) } finally { $stream.Dispose() }
        }
        return [Convert]::ToHexString($hash.GetHashAndReset())
    } finally { $hash.Dispose() }
}
function Get-StampOrUncertain([string]$Root,[string[]]$Modules,[string]$Identity) {
    try { return Get-InputStamp $Root $Modules $Identity }
    catch { Write-Warning "无法确认缓存，本次执行增量构建：$($_.Exception.Message)"; return '' }
}
function Get-OutputStamp([string[]]$Paths) {
    $records = [Collections.Generic.List[string]]::new()
    foreach ($path in $Paths) {
        if ([IO.Directory]::Exists($path)) {
            foreach ($filePath in [IO.Directory]::EnumerateFiles($path,'*',[IO.SearchOption]::AllDirectories)) {
                $file=[IO.FileInfo]::new($filePath)
                $records.Add("$($file.FullName)`0$($file.Length)`0$($file.LastWriteTimeUtc.Ticks)")
            }
        } elseif ([IO.File]::Exists($path)) {
            $file=[IO.FileInfo]::new($path)
            $records.Add("$($file.FullName)`0$($file.Length)`0$($file.LastWriteTimeUtc.Ticks)")
        } else { return '' }
    }
    $records.Sort([StringComparer]::Ordinal)
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes(($records -join "`n"))))
}
try {
    $entry = $env:ZED_JAVA_ENTRY | ConvertFrom-Json
    Remove-Item Env:ZED_JAVA_ENTRY
    $root = [IO.Path]::GetFullPath([string]$entry.root)
    Set-Location -LiteralPath $root
    $legacyPath = Join-Path $root '.zed/run-config.json'
    if (Test-Path -LiteralPath $legacyPath -PathType Leaf) {
        try { $legacy = Get-Content -LiteralPath $legacyPath -Raw | ConvertFrom-Json }
        catch { throw '无法读取项目私密运行配置，请检查 .zed/run-config.json。' }
        if ($legacy.env) { foreach ($variable in $legacy.env.PSObject.Properties) {
            if (@($entry.envKeys) -notcontains $variable.Name) { [Environment]::SetEnvironmentVariable($variable.Name,[string]$variable.Value,'Process') }
        } }
    }
    $java = [string]$entry.java
    if (![IO.Path]::IsPathFullyQualified($java) -or !(Test-Path -LiteralPath $java -PathType Container)) {
        throw '请在 java.runtime 中配置 JDK 的绝对目录。'
    }
    $env:JAVA_HOME=$java
    $java=Join-Path $java $runnerJavaRelativePath
    if (!(Test-Path -LiteralPath $java -PathType Leaf)) { throw "JDK 缺少 $runnerJavaRelativePath：$env:JAVA_HOME" }
    $env:PATH=(Join-Path $env:JAVA_HOME 'bin')+[IO.Path]::PathSeparator+$env:PATH
    $javaArguments=@('-Dfile.encoding=UTF-8','-Dstdout.encoding=UTF-8','-Dstderr.encoding=UTF-8')+@($entry.jvmArgs)
    if ($IsWindows -and [Diagnostics.FileVersionInfo]::GetVersionInfo($java).FileMajorPart -ge 17 -and !@($javaArguments | Where-Object { $_ -like '--enable-native-access*' }).Count) { $javaArguments+='--enable-native-access=ALL-UNNAMED' }
    if ($entry.module) {
        $modulePom=[IO.Path]::GetFullPath([string]$entry.module)
        $moduleDirectory=Split-Path $modulePom -Parent
        if (!(Test-RunnerPathWithin $moduleDirectory $root)) { throw '启动模块必须位于当前项目内。' }
        $maven=[string]$entry.maven
        if (![IO.Path]::IsPathFullyQualified($maven) -or !(Test-Path -LiteralPath $maven -PathType Leaf)) {
            throw '请在 java.build_tool 中配置 Maven/MVND 可执行文件的绝对路径。'
        }
        $target=Join-Path $moduleDirectory 'target'
        $classpathFile=Join-Path $target 'zed-run-classpath.txt'
        $stateFile=Join-Path $target 'zed-run-state.json'
        $classes=Join-Path $target 'classes'
        $mainClassFile=Join-Path $classes (([string]$entry.class).Replace('.','/')+'.class')
        $identity="$java|$((Get-Item -LiteralPath $java).LastWriteTimeUtc.Ticks)|$maven|$env:MAVEN_OPTS|$env:JAVA_TOOL_OPTIONS|$env:JDK_JAVA_OPTIONS"
        $lockKey=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($(if ($IsWindows) { $root.ToLowerInvariant() } else { $root }))))
        $mutexName = if ($IsWindows) { "Local\zed-java-build-$lockKey" } else { "zed-java-build-$lockKey" }
        $mutex=[Threading.Mutex]::new($false,$mutexName)
        try { $locked=$mutex.WaitOne([TimeSpan]::FromSeconds(5)) } catch [Threading.AbandonedMutexException] { $locked=$true }
        if (!$locked) { throw '当前项目正在构建，请等待后再运行。' }
        $state=$null
        if (Test-Path -LiteralPath $stateFile) { try { $state=Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json } catch { Write-Warning '运行缓存损坏，将重新增量构建。' } }
        $fresh=$false
        if ($state -and $state.version -eq 1 -and (Test-Path -LiteralPath $classpathFile) -and (Test-Path -LiteralPath $mainClassFile)) {
            $stamp=Get-StampOrUncertain $root @($state.modules) $identity
            $fresh=$stamp -and $stamp -eq $state.inputs -and (Get-OutputStamp @($state.paths)) -eq $state.outputs -and (Get-FileHash -LiteralPath $classpathFile).Hash -eq $state.classpathHash
        }
        if (!$fresh) {
            $before=Get-StampOrUncertain $root @() $identity
            $relativeModule=[IO.Path]::GetRelativePath($root,$moduleDirectory)
            $buildArguments=@('-f',$modulePom)
            if ((Test-Path -LiteralPath (Join-Path $root 'pom.xml')) -and $relativeModule -ne '.') { $buildArguments=@('-f',(Join-Path $root 'pom.xml'),'-pl',$relativeModule,'-am') }
            if ([IO.Path]::GetFileNameWithoutExtension($maven) -eq 'mvnd') { $buildArguments+="-Dmvnd.java.home=$env:JAVA_HOME" }
            Write-Host '运行前增量构建：compile + 解析运行依赖（不执行 package / clean）' -ForegroundColor Cyan
            & $maven @buildArguments '-B' '-T' '8' '-DskipTests' 'compile' 'dependency:build-classpath' '-DincludeScope=runtime' '-Dmdep.outputFile=target/zed-run-classpath.txt' '-Dmdep.regenerateFile=true'
            if ($LASTEXITCODE -ne 0) { throw "增量构建失败（退出码 $LASTEXITCODE），未启动应用。" }
            if (!(Test-Path -LiteralPath $classpathFile) -or !(Test-Path -LiteralPath $mainClassFile)) { throw '未生成启动类或 classpath，请检查 Main 类和模块。' }
            $paths=@($classes)+@(([IO.File]::ReadAllText($classpathFile).Trim() -split [regex]::Escape([string][IO.Path]::PathSeparator)) | Where-Object { $_ })
            $modules=@($moduleDirectory)+@($paths | Where-Object { $_.EndsWith('\target\classes') -or $_.EndsWith('/target/classes') } | ForEach-Object { Split-Path (Split-Path $_ -Parent) -Parent })
            $modules=@($modules | Sort-Object -Unique)
            $after=Get-StampOrUncertain $root @() $identity
            if ($before -and $after -and $before -ne $after) { throw '构建期间项目文件发生变化，请保存后重新运行。' }
            if ($before -and $after) {
                $inputs=Get-StampOrUncertain $root $modules $identity
                $outputs=Get-OutputStamp $paths
                if ($inputs -and $outputs) { @{version=1;inputs=$inputs;outputs=$outputs;modules=$modules;paths=$paths;classpathHash=(Get-FileHash -LiteralPath $classpathFile).Hash} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $stateFile -Encoding utf8 }
            }
        } else { Write-Host '源码、依赖及编译产物未变化，直接启动。' -ForegroundColor Green }
        $classpath=$classes+[IO.Path]::PathSeparator+[IO.File]::ReadAllText($classpathFile).Trim()
        $javaArguments+=@('-cp',$classpath,[string]$entry.class)
        $mutex.ReleaseMutex();$locked=$false
    } else {
        $target=Join-Path $root '.zed/java-run'
        New-Item -ItemType Directory -Path $target -Force | Out-Null
        $javaArguments+=[string]$entry.file
    }
    if ($BuildOnly) { Write-Host '运行前构建检查完成，未启动应用。'; exit 0 }
    if ($entry.module) {
        # The Windows Java launcher decodes argument files using the system ANSI
        # code page, independently of file.encoding and the console code page.
        $argumentFile=Join-Path $target ('zed-java-arguments-'+[Guid]::NewGuid().ToString('N')+'.txt')
        $lines=@($javaArguments | Where-Object { $null -ne $_ } | ForEach-Object { '"'+([string]$_).Replace('\','\\').Replace('"','\"').Replace("`n",'\n').Replace("`r",'\r')+'"' })
        [Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance)
        $codePage=if ($IsWindows) { [int][Microsoft.Win32.Registry]::GetValue('HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Nls\CodePage','ACP',65001) } else { 65001 }
        $encoding=if ($codePage -eq 65001) { [Text.UTF8Encoding]::new($false,$true) } else { [Text.Encoding]::GetEncoding($codePage,[Text.EncoderFallback]::ExceptionFallback,[Text.DecoderFallback]::ExceptionFallback) }
        try {
            [IO.File]::WriteAllLines($argumentFile,$lines,$encoding)
            $javaArguments=@('@'+$argumentFile)
        } catch [Text.EncoderFallbackException] {
            if (!$IsWindows) { throw }
            # Windows' ANSI argument files cannot represent every Unicode path.
            # Keep the original arguments so PowerShell passes them as UTF-16.
            if (Test-Path -LiteralPath $argumentFile) { Remove-Item -LiteralPath $argumentFile -Force }
            $argumentFile=$null
        }
    }
    $javaArguments+=@($entry.args)
    Write-Host "启动 Main：$($entry.class)" -ForegroundColor Cyan
    & $java @javaArguments
    exit $LASTEXITCODE
} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
finally {
    if ($locked) { $mutex.ReleaseMutex() }
    if ($mutex) { $mutex.Dispose() }
    if ($argumentFile -and (Test-Path -LiteralPath $argumentFile)) { Remove-Item -LiteralPath $argumentFile -Force }
}
