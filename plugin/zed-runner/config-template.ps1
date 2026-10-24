$ErrorActionPreference = 'Stop'
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
    return ''
}
$jdk = if ($env:JAVA_HOME -and (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME 'bin/java.exe') -PathType Leaf)) {
    [IO.Path]::GetFullPath($env:JAVA_HOME)
} else { '' }
$maven = Existing-File @($env:MVND_EXE, $env:MAVEN_EXE)
if (!$maven) { $maven = In-Directory $env:MVND_HOME @('bin/mvnd.exe','mvnd.exe') }
if (!$maven) { $maven = In-Directory $env:MAVEN_HOME @('bin/mvn.cmd','mvn.cmd') }
if (!$maven) { $maven = On-Path 'mvnd' }
if (!$maven) { $maven = On-Path 'mvn' }
$python = Existing-File @($env:PYTHON_EXECUTABLE, $env:PYTHON_EXE)
if (!$python) { $python = In-Directory $env:VIRTUAL_ENV @('Scripts/python.exe') }
if (!$python) { $python = In-Directory $env:PYTHON_HOME @('python.exe','Scripts/python.exe') }
$uv = Existing-File @($env:UV_EXE)
if (!$uv) { $uv = In-Directory $env:UV_HOME @('uv.exe','bin/uv.exe') }
if (!$uv) { $uv = On-Path 'uv' }
$go = Existing-File @($env:GO_EXE)
if (!$go) { $go = In-Directory $env:GOROOT @('bin/go.exe') }
if (!$go) { $go = On-Path 'go' }
$npm = Existing-File @($env:NPM_EXE)
if (!$npm) { $npm = In-Directory $env:NODE_HOME @('npm.cmd','bin/npm.cmd') }
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
