param([Parameter(Mandatory=$true)][string]$Root)
$ErrorActionPreference = 'Stop'
$Root = [IO.Path]::GetFullPath($Root)
$configPath = Join-Path $Root '.zed/run-environments.json'
$config = if (Test-Path -LiteralPath $configPath) { Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json } else { $null }
$entries = [Collections.Generic.List[object]]::new()
$warnings = [Collections.Generic.List[string]]::new()
$files = @()
try {
    $git = Get-Command git -CommandType Application -ErrorAction Stop
    $files = @(& $git.Source -C $Root ls-files --cached --others --exclude-standard 2>$null | Where-Object { $_ })
    if ($LASTEXITCODE -ne 0) { $files = @() }
} catch { $files = @() }
if (!$files.Count) {
    $files = @(Get-ChildItem -LiteralPath $Root -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -notmatch '[\\/](?:target|build|node_modules|\.git|\.venv)[\\/]' } |
        ForEach-Object { [IO.Path]::GetRelativePath($Root, $_.FullName).Replace('\','/') })
}
function Runtime([string]$Language, [string]$Default) {
    $settings = $config.environments.$Language
    if ($settings -and $settings.runtime) { return [string]$settings.runtime }
    return $Default
}
function Environment([string]$Language) {
    $result = @{}
    $settings = $config.environments.$Language
    if ($settings -and $settings.env) {
        foreach ($property in $settings.env.PSObject.Properties) { $result[$property.Name] = [string]$property.Value }
    }
    return $result
}
function Add-Entry([string]$Id, [string]$Language, [string]$File, [string]$Label, [string]$Command,
                   [string[]]$Arguments, [hashtable]$Variables, [string[]]$Extensions, $DebugConfig) {
    $task = @{ label=$Label; command=$Command; args=@($Arguments); cwd=$Root; env=$Variables;
               save='all'; show_command=$false; show_summary=$true }
    $entry = @{ id=$Id; language=$Language; file=$File.Replace('\','/');
                matching_extensions=@($Extensions); task=$task }
    if ($DebugConfig) { $entry.debug = $DebugConfig }
    $entries.Add($entry)
}
$goRoots = @($files | Where-Object { $_ -match '(^|/)go\.mod$' })
foreach ($moduleFile in $goRoots) {
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
        $runtime = Runtime 'go' 'go'
        $environment = Environment 'go'
        $debug = @{ adapter='Delve'; label="Go · $directory"; request='launch'; mode='debug';
                    program=$absoluteDirectory; cwd=$absoluteDirectory; env=$environment }
        Add-Entry "go:$mainFile" 'go' $mainFile "Go · $directory" $runtime @('run','.') $environment @('go') $debug
        $entries[$entries.Count-1].task.cwd = $absoluteDirectory
    }
}
foreach ($file in @($files | Where-Object { $_ -like '*.py' -and $_ -notmatch '(^|/)(test_|conftest\.py)' })) {
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    if ([IO.File]::ReadAllText($absolute) -notmatch '__name__\s*==\s*[''\"]__main__[''\"]') { continue }
    $runtime = Runtime 'python' 'python'
    $environment = Environment 'python'
    $debug = @{ adapter='Debugpy'; label="Python · $file"; request='launch';
                program=$absolute; cwd=$Root; env=$environment }
    Add-Entry "python:$file" 'python' $file "Python · $file" $runtime @($absolute) $environment @('py') $debug
}
foreach ($file in @($files | Where-Object { $_ -like '*.java' })) {
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    $content = [IO.File]::ReadAllText($absolute)
    if ($content -notmatch 'public\s+static\s+void\s+main\s*\(') { continue }
    $package = if ($content -match '(?m)^\s*package\s+([\w.]+)\s*;') { $Matches[1] } else { '' }
    $class = [IO.Path]::GetFileNameWithoutExtension($file)
    $main = if ($package) { "$package.$class" } else { $class }
    $module = $null
    $parent = [IO.Path]::GetDirectoryName($absolute)
    while ($parent -and $parent.StartsWith($Root,[StringComparison]::OrdinalIgnoreCase)) {
        $pom = Join-Path $parent 'pom.xml'
        if (Test-Path -LiteralPath $pom) { $module = $pom; break }
        $parent = [IO.Path]::GetDirectoryName($parent)
    }
    $settings = $config.environments.java
    $environment = Environment 'java'
    $payload = @{ root=$Root; file=$absolute; class=$main; module=$module; java=(Runtime 'java' 'java');
                  maven=$(if ($settings -and $settings.build_tool) { [string]$settings.build_tool } else { '' });
                  args=@($(if ($settings) { $settings.args }));
                  jvmArgs=@($(if ($settings) { $settings.jvm_args })); envKeys=@($environment.Keys) } | ConvertTo-Json -Compress -Depth 10
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
                vmArgs=@('-Dfile.encoding=UTF-8') + @($(if ($settings) { $settings.jvm_args })) }
    if ($settings -and $settings.runtime) {
        $javaRuntime = [string]$settings.runtime
        if (Test-Path -LiteralPath $javaRuntime -PathType Container) {
            $debug.javaExec = Join-Path $javaRuntime 'bin/java.exe'
        } elseif (Test-Path -LiteralPath $javaRuntime -PathType Leaf) { $debug.javaExec = $javaRuntime }
    }
    Add-Entry "java:$file" 'java' $file "Java · $main" 'pwsh' @('-NoLogo','-NoProfile','-File',"$PSScriptRoot/scripts/run_java.ps1") $environment @('java') $debug
}
foreach ($file in @($files | Where-Object { $_ -match '(^|/)package\.json$' })) {
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    try { $package = Get-Content -LiteralPath $absolute -Raw | ConvertFrom-Json } catch {
        $warnings.Add("$file 无法解析：$($_.Exception.Message)")
        continue
    }
    $directory = Split-Path $absolute -Parent
    foreach ($script in @($package.scripts.PSObject.Properties)) {
        if (!$script) { continue }
        $runtime = Runtime 'javascript' 'npm'
        Add-Entry "javascript:$file`:$($script.Name)" 'javascript' $file "npm · $($script.Name) · $file" $runtime @('run',$script.Name) (Environment 'javascript') @('js','jsx','ts','tsx','json') $null
        $entries[$entries.Count-1].task.cwd = $directory
    }
}
foreach ($file in @($files | Where-Object { $_ -match '(^|/)Cargo\.toml$' })) {
    $absolute = Join-Path $Root $file
    $directory = Split-Path $absolute -Parent
    $mainFile = Join-Path $directory 'src/main.rs'
    if (!(Test-Path -LiteralPath $mainFile)) { continue }
    $runtime = Runtime 'rust' 'cargo'
    Add-Entry "rust:$file" 'rust' $file "Rust · $file" $runtime @('run','--manifest-path',$absolute) (Environment 'rust') @('rs') $null
    $entries[$entries.Count-1].task.cwd = $directory
}
foreach ($file in @($files | Where-Object { $_ -like '*.csproj' })) {
    $absolute = Join-Path $Root $file
    if (!(Test-Path -LiteralPath $absolute)) { continue }
    $directory = Split-Path $absolute -Parent
    $runtime = Runtime 'csharp' 'dotnet'
    Add-Entry "csharp:$file" 'csharp' $file ".NET · $file" $runtime @('run','--project',$absolute) (Environment 'csharp') @('cs','csproj') $null
    $entries[$entries.Count-1].task.cwd = $directory
}
foreach ($item in @($config.entries)) {
    if (!$item) { continue }
    $entry = $item | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
    if ($entry.id -and $entry.task -and $entry.file) { $entries.Add($entry) }
}
@{ version=1; entries=@($entries.ToArray()); warnings=@($warnings.ToArray()) } | ConvertTo-Json -Depth 50 -Compress -EscapeHandling EscapeNonAscii
