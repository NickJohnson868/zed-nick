param([Parameter(Mandatory=$true)][string]$Root)
$ErrorActionPreference = 'Stop'
$Root = [IO.Path]::GetFullPath($Root)
$configPath = Join-Path $Root '.zed/run-environments.json'
$config = if (Test-Path -LiteralPath $configPath) { Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json } else { $null }
$entries = [Collections.Generic.List[object]]::new()
$warnings = [Collections.Generic.List[string]]::new()
$powerShellExecutable = (Get-Process -Id $PID).Path
. "$PSScriptRoot/scripts/platform.ps1"
. "$PSScriptRoot/scripts/workspace_files.ps1"
try {
    $files = @(Get-WorkspaceFiles $Root)
} catch {
    $files = @()
    $warnings.Add("自动入口扫描失败：$($_.Exception.Message)")
}
function Configured-File([string]$Value, [string]$Name) {
    if (!$Value) { return $null }
    if (![IO.Path]::IsPathFullyQualified($Value) -or !(Test-Path -LiteralPath $Value -PathType Leaf)) {
        $warnings.Add("$Name 必须是现有文件的绝对路径；已跳过对应入口")
        return $null
    }
    return [IO.Path]::GetFullPath($Value)
}
function Runtime([string]$Language, [string]$Default = '') {
    $configured = [string]$config.environments.$Language.runtime
    if (!$configured) { return $Default }
    return Configured-File $configured "$Language.runtime"
}
function Java-Runtime {
    $jdkDirectory = [string]$config.environments.java.runtime
    if (!$jdkDirectory) { return $null }
    if (![IO.Path]::IsPathFullyQualified($jdkDirectory) -or
        !(Test-Path -LiteralPath $jdkDirectory -PathType Container) -or
        !(Test-Path -LiteralPath (Join-Path $jdkDirectory $runnerJavaRelativePath) -PathType Leaf)) {
        $warnings.Add("java.runtime 必须是包含 $runnerJavaRelativePath 的 JDK 绝对目录；已跳过 Java 入口")
        return $null
    }
    return [IO.Path]::GetFullPath($jdkDirectory)
}
function Maven-Runtime {
    $path = [string]$config.environments.java.build_tool
    if (!$path) { return $null }
    if ([IO.Path]::IsPathFullyQualified($path) -and (Test-Path -LiteralPath $path -PathType Container)) {
        foreach ($name in $(if ($IsWindows) { @('bin/mvnd.exe','bin/mvn.cmd','mvnd.exe','mvn.cmd') } else { @('bin/mvnd','bin/mvn','mvnd','mvn') })) {
            $candidate = Join-Path $path $name
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return [IO.Path]::GetFullPath($candidate) }
        }
    }
    return Configured-File $path 'java.build_tool'
}
function Environment([string]$Language) {
    $result = @{}
    $settings = $config.environments.$Language
    if ($settings -and $settings.env) {
        foreach ($property in $settings.env.PSObject.Properties) { $result[$property.Name] = [string]$property.Value }
    }
    return $result
}
function Local-Python([string]$ScriptPath) {
    $directory = [IO.Path]::GetDirectoryName($ScriptPath)
    while ($directory -and (Test-RunnerPathWithin $directory $Root)) {
        foreach ($name in @('.venv', 'venv')) {
            $candidate = Join-Path $directory "$name/$runnerPythonRelativePath"
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
        }
        if ($directory -eq $Root) { break }
        $directory = [IO.Path]::GetDirectoryName($directory)
    }
    return $null
}
function Python-Project([string]$ScriptPath) {
    $directory = [IO.Path]::GetDirectoryName($ScriptPath)
    while ($directory -and (Test-RunnerPathWithin $directory $Root)) {
        if ((Test-Path -LiteralPath (Join-Path $directory 'pyproject.toml') -PathType Leaf) -or
            (Test-Path -LiteralPath (Join-Path $directory 'requirements.txt') -PathType Leaf)) {
            return $directory
        }
        if ($directory -eq $Root) { break }
        $directory = [IO.Path]::GetDirectoryName($directory)
    }
    return $null
}
function Add-Entry([string]$Id, [string]$Language, [string]$File, [string]$Label, [string]$Command,
                   [string[]]$Arguments, [hashtable]$Variables, [string[]]$Extensions, $DebugConfig) {
    if (!$IsWindows -and !$Variables.ContainsKey('PATH')) {
        $directories = @([IO.Path]::GetDirectoryName($Command),'/opt/homebrew/bin','/usr/local/bin') + @($env:PATH -split [regex]::Escape([string][IO.Path]::PathSeparator))
        $Variables.PATH = (@($directories | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Container) } | Select-Object -Unique) -join [IO.Path]::PathSeparator)
    }
    $taskCommand = "& '" + $Command.Replace("'", "''") + "'"
    $taskArguments = @($Arguments | ForEach-Object { "'" + $_.Replace("'", "''") + "'" })
    $taskArguments += '; exit $LASTEXITCODE'
    $task = @{ label=$Label; command=$taskCommand; args=$taskArguments; cwd=$Root; env=$Variables;
               shell=@{ with_arguments=@{ program=$powerShellExecutable; args=@('-NoLogo','-NoProfile') } };
               save='all'; show_command=$false; show_summary=$true }
    $entry = @{ id=$Id; language=$Language; file=$File.Replace('\','/');
                matching_extensions=@($Extensions); task=$task }
    if ($DebugConfig) { $entry.debug = $DebugConfig }
    $entries.Add($entry)
}
$goRoots = @($files | Where-Object { $_ -match '(^|/)go\.mod$' })
$goRuntime = Runtime 'go'
foreach ($moduleFile in $goRoots) {
    if (!$goRuntime) { continue }
    $module = Split-Path $moduleFile -Parent
    if (!$module) { $module = '.' }
    $mainFiles = @($files | Where-Object {
        $_ -like "$($module.Replace('\','/').TrimEnd('.'))*" -and $_ -like '*.go' -and
        (Test-Path -LiteralPath (Join-Path $Root $_)) -and
        [IO.File]::ReadAllText((Join-Path $Root $_)) -match '(?m)^\s*func\s+main\s*\('
    })
    foreach ($mainFile in $mainFiles) {
        $directory = Split-Path $mainFile -Parent
        $absoluteDirectory = if ($directory) { Join-Path $Root $directory } else { $Root }
        $environment = Environment 'go'
        $debug = @{ adapter='Delve'; label="Go · $directory"; request='launch'; mode='debug';
                    program=$absoluteDirectory; cwd=$absoluteDirectory; env=$environment }
        Add-Entry "go:$mainFile" 'go' $mainFile "Go · $directory" $goRuntime @('run','.') $environment @('go') $debug
        $entries[$entries.Count-1].task.cwd = $absoluteDirectory
    }
}
$skippedPython = [Collections.Generic.List[string]]::new()
$pythonBase = Configured-File ([string]$config.environments.python.runtime) 'python.runtime'
$uvRuntime = Configured-File ([string]$config.environments.python.uv) 'python.uv'
foreach ($file in @($files | Where-Object { $_ -like '*.py' -and $_ -notmatch '(^|/)(test_|conftest\.py)' })) {
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    if ([IO.File]::ReadAllText($absolute) -notmatch '__name__\s*==\s*[''\"]__main__[''\"]') { continue }
    $projectDirectory = Python-Project $absolute
    $hasPyproject = $projectDirectory -and (Test-Path -LiteralPath (Join-Path $projectDirectory 'pyproject.toml') -PathType Leaf)
    $hasRequirements = $projectDirectory -and (Test-Path -LiteralPath (Join-Path $projectDirectory 'requirements.txt') -PathType Leaf)
    if ($hasPyproject -and !$uvRuntime) {
        if (!$warnings.Contains('扫描 pyproject.toml 需要在 python.uv 中配置 uv 的绝对路径')) {
            $warnings.Add('扫描 pyproject.toml 需要在 python.uv 中配置 uv 的绝对路径')
        }
        $skippedPython.Add($file)
        continue
    }
    if ($hasRequirements -and !$pythonBase) {
        if (!$warnings.Contains('扫描 requirements.txt 需要在 python.runtime 中配置 Python 可执行文件的绝对路径')) {
            $warnings.Add('扫描 requirements.txt 需要在 python.runtime 中配置 Python 可执行文件的绝对路径')
        }
        $skippedPython.Add($file)
        continue
    }
    if (!$hasPyproject -and !$hasRequirements -and !$pythonBase) {
        $skippedPython.Add($file)
        continue
    }
    $runtime = Local-Python $absolute
    if (!$runtime -and $projectDirectory) { $runtime = Join-Path $projectDirectory ".venv/$runnerPythonRelativePath" }
    if (!$runtime) { $runtime = $pythonBase }
    if (!$runtime) {
        $skippedPython.Add($file)
        continue
    }
    $environment = Environment 'python'
    $debug = @{ adapter='Debugpy'; label="Python · $file"; request='launch';
                program=$absolute; cwd=$Root; env=$environment; python=$runtime }
    if ($projectDirectory) {
        $prepareArgs = @('-NoLogo','-NoProfile','-File',"$PSScriptRoot/scripts/run_python.ps1",'-Project',$projectDirectory,'-Python',$runtime)
        if ($uvRuntime) { $prepareArgs += @('-Uv',$uvRuntime) }
        if ($pythonBase) { $prepareArgs += @('-BasePython',$pythonBase) }
        $debug.build = @{ command=$powerShellExecutable; args=@($prepareArgs); cwd=$projectDirectory; env=$environment }
        Add-Entry "python:$file" 'python' $file "Python · $file" $powerShellExecutable @($prepareArgs + @('-Script',$absolute)) $environment @('py') $debug
    } else {
        Add-Entry "python:$file" 'python' $file "Python · $file" $runtime @($absolute) $environment @('py') $debug
    }
}
if ($skippedPython.Count) {
    $warnings.Add("已跳过 $($skippedPython.Count) 个缺少明确工具路径的 Python 入口（例如 $($skippedPython[0])）；未使用全局 Python")
}
$javaHome = Java-Runtime
$mavenRuntime = Maven-Runtime
foreach ($file in @($files | Where-Object { $_ -like '*.java' })) {
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    $content = [IO.File]::ReadAllText($absolute)
    if ($content -notmatch 'public\s+static\s+void\s+main\s*\(') { continue }
    if (!$javaHome) {
        if (!$warnings.Contains('扫描 Java Main 需要在 java.runtime 中配置 JDK 绝对目录')) {
            $warnings.Add('扫描 Java Main 需要在 java.runtime 中配置 JDK 绝对目录')
        }
        break
    }
    $package = if ($content -match '(?m)^\s*package\s+([\w.]+)\s*;') { $Matches[1] } else { '' }
    $class = [IO.Path]::GetFileNameWithoutExtension($file)
    $main = if ($package) { "$package.$class" } else { $class }
    $module = $null
    $parent = [IO.Path]::GetDirectoryName($absolute)
    while ($parent -and (Test-RunnerPathWithin $parent $Root)) {
        $pom = Join-Path $parent 'pom.xml'
        if (Test-Path -LiteralPath $pom) { $module = $pom; break }
        $parent = [IO.Path]::GetDirectoryName($parent)
    }
    if ($file -notmatch '(^|/)src/main/java/') { $module = $null }
    if ($module -and !$mavenRuntime) {
        if (!$warnings.Contains('扫描 Maven 模块的 Java Main 需要在 java.build_tool 中配置 Maven/MVND 绝对路径')) {
            $warnings.Add('扫描 Maven 模块的 Java Main 需要在 java.build_tool 中配置 Maven/MVND 绝对路径')
        }
        continue
    }
    $settings = $config.environments.java
    $environment = Environment 'java'
    $programArguments = @($settings.args | Where-Object { $_ -is [string] -and $_ })
    $jvmArguments = @($settings.jvm_args | Where-Object { $_ -is [string] -and $_ })
    $payload = @{ root=$Root; file=$absolute; class=$main; module=$module; java=$javaHome;
                  maven=$mavenRuntime;
                  args=$programArguments; jvmArgs=$jvmArguments; envKeys=@($environment.Keys) } | ConvertTo-Json -Compress -Depth 10
    $environment.ZED_JAVA_ENTRY = $payload
    $debugEnvironment = Environment 'java'
    $privateConfigPath = Join-Path $Root '.zed/run-config.json'
    if (Test-Path -LiteralPath $privateConfigPath) {
        $privateConfig = Get-Content -LiteralPath $privateConfigPath -Raw | ConvertFrom-Json
        if ($privateConfig.env) {
            foreach ($variable in $privateConfig.env.PSObject.Properties) {
                if (!$debugEnvironment.ContainsKey($variable.Name)) { $debugEnvironment[$variable.Name] = [string]$variable.Value }
            }
        }
    }
    $debug = @{ adapter='Java'; label="Java · $main"; request='launch'; mainClass=$main;
                cwd=$Root; env=$debugEnvironment; encoding='UTF-8';
                vmArgs=@('-Dfile.encoding=UTF-8','-Dstdout.encoding=UTF-8','-Dstderr.encoding=UTF-8') + $jvmArguments }
    if ($programArguments.Count) { $debug.args = $programArguments }
    $debug.javaExec = Join-Path $javaHome $runnerJavaRelativePath
    if ($module) {
        $moduleDirectory = Split-Path $module -Parent
        $classpathFile = Join-Path $moduleDirectory 'target/zed-run-classpath.txt'
        if ((Test-Path -LiteralPath $classpathFile -PathType Leaf) -and
            (Test-Path -LiteralPath (Join-Path $moduleDirectory 'target/classes') -PathType Container)) {
            $classpathEntries = @([IO.Path]::Combine($moduleDirectory,'target','classes')) +
                @(([IO.File]::ReadAllText($classpathFile).Trim() -split [regex]::Escape([string][IO.Path]::PathSeparator)) | Where-Object { $_ })
            if (!@($classpathEntries | Where-Object { !(Test-Path -LiteralPath $_) }).Count) {
                $debug.classPaths = $classpathEntries
            }
        }
        $debug.build = @{ command=$powerShellExecutable; args=@('-NoLogo','-NoProfile','-File',"$PSScriptRoot/scripts/run_java.ps1",'-BuildOnly'); cwd=$Root; env=$environment }
    } else {
        $debug = $null
    }
    Add-Entry "java:$file" 'java' $file "Java · $main" $powerShellExecutable @('-NoLogo','-NoProfile','-File',"$PSScriptRoot/scripts/run_java.ps1") $environment @('java') $debug
}
$npmRuntime = Runtime 'javascript'
foreach ($file in @($files | Where-Object { $_ -match '(^|/)package\.json$' })) {
    if (!$npmRuntime) { continue }
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    try { $package = Get-Content -LiteralPath $absolute -Raw | ConvertFrom-Json } catch {
        $warnings.Add("$file 无法解析：$($_.Exception.Message)")
        continue
    }
    $directory = Split-Path $absolute -Parent
    foreach ($script in @($package.scripts.PSObject.Properties)) {
        if (!$script) { continue }
        Add-Entry "javascript:$file`:$($script.Name)" 'javascript' $file "npm · $($script.Name) · $file" $npmRuntime @('run',$script.Name) (Environment 'javascript') @('js','jsx','ts','tsx','json') $null
        $entries[$entries.Count-1].task.cwd = $directory
    }
}
foreach ($item in @($config.entries)) {
    if (!$item) { continue }
    $entry = $item | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
    if ($entry.id -and $entry.task -and $entry.file) { $entries.Add($entry) }
}
@{ version=1; entries=@($entries.ToArray()); warnings=@($warnings.ToArray()) } | ConvertTo-Json -Depth 50 -Compress -EscapeHandling EscapeNonAscii
