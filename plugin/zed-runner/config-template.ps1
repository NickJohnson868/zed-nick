$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/scripts/platform.ps1"
function Existing-File([string[]]$Candidates) {
    foreach ($candidate in $Candidates) {
        if ($candidate -and [IO.Path]::IsPathFullyQualified($candidate) -and
            (Test-Path -LiteralPath $candidate -PathType Leaf)) { return [IO.Path]::GetFullPath($candidate) }
    }
    return ''
}
function In-Directory([string]$Directory, [string[]]$Names) {
    if (!$Directory -or ![IO.Path]::IsPathFullyQualified($Directory)) { return '' }
    foreach ($name in $Names) {
        $candidate = Join-Path $Directory $name
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return [IO.Path]::GetFullPath($candidate) }
    }
    return ''
}
function On-Path([string]$Name) {
    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return [string]$command.Source }
    if (!$IsWindows) {
        foreach ($directory in @('/opt/homebrew/bin','/usr/local/bin','/usr/bin')) {
            $candidate = Join-Path $directory $Name
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
        }
    }
    return ''
}
$jdk = if ($env:JAVA_HOME -and (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME $runnerJavaRelativePath) -PathType Leaf)) {
    [IO.Path]::GetFullPath($env:JAVA_HOME)
} else { '' }
$maven = Existing-File @($env:MVND_EXE, $env:MAVEN_EXE)
if (!$maven) { $maven = In-Directory $env:MVND_HOME $(if ($IsWindows) { @('bin/mvnd.exe','mvnd.exe') } else { @('bin/mvnd','mvnd') }) }
if (!$maven) { $maven = In-Directory $env:MAVEN_HOME $(if ($IsWindows) { @('bin/mvn.cmd','mvn.cmd') } else { @('bin/mvn','mvn') }) }
if (!$maven) { $maven = On-Path 'mvnd' }
if (!$maven) { $maven = On-Path 'mvn' }
$python = Existing-File @($env:PYTHON_EXECUTABLE, $env:PYTHON_EXE)
if (!$python) { $python = In-Directory $env:VIRTUAL_ENV @($runnerPythonRelativePath) }
if (!$python) { $python = In-Directory $env:PYTHON_HOME $(if ($IsWindows) { @('python.exe','Scripts/python.exe') } else { @('bin/python3','bin/python') }) }
if (!$python) { $python = On-Path 'python3' }
if (!$python) { $python = On-Path 'python' }
if (!$jdk -and $IsMacOS -and (Test-Path -LiteralPath '/usr/libexec/java_home')) {
    $detectedHome = & /usr/libexec/java_home 2>$null
    if ($LASTEXITCODE -eq 0 -and $detectedHome -and (Test-Path -LiteralPath (Join-Path $detectedHome $runnerJavaRelativePath))) { $jdk = [string]$detectedHome }
}
if (!$jdk) {
    $java = On-Path 'java'
    if ($java) {
        $resolved = (Get-Item -LiteralPath $java).ResolveLinkTarget($true)
        if ($resolved) { $java = $resolved.FullName }
        $candidate = Split-Path (Split-Path $java -Parent) -Parent
        if (Test-Path -LiteralPath (Join-Path $candidate $runnerJavaRelativePath)) { $jdk = $candidate }
    }
}
$uv = Existing-File @($env:UV_EXE)
if (!$uv) { $uv = In-Directory $env:UV_HOME $(if ($IsWindows) { @('uv.exe','bin/uv.exe') } else { @('bin/uv','uv') }) }
if (!$uv) { $uv = On-Path 'uv' }
$go = Existing-File @($env:GO_EXE)
if (!$go) { $go = In-Directory $env:GOROOT $(if ($IsWindows) { @('bin/go.exe') } else { @('bin/go') }) }
if (!$go) { $go = On-Path 'go' }
$npm = Existing-File @($env:NPM_EXE)
if (!$npm) { $npm = In-Directory $env:NODE_HOME $(if ($IsWindows) { @('npm.cmd','bin/npm.cmd') } else { @('bin/npm','npm') }) }
if (!$npm) { $npm = On-Path 'npm' }
@{
    environments = @{
        java = @{ runtime=$jdk; build_tool=$maven; env=@{}; args=@(); jvm_args=@() }
        python = @{ runtime=$python; uv=$uv; env=@{} }
        go = @{ runtime=$go; env=@{} }
        javascript = @{ runtime=$npm; env=@{} }
    }
    entries = @()
} | ConvertTo-Json -Depth 10 -EscapeHandling EscapeNonAscii
