if ($PSVersionTable.PSVersion.Major -lt 6) {
    $IsWindows = $true
    $IsMacOS = $false
    $IsLinux = $false
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    $OutputEncoding = [Console]::OutputEncoding
}

function Test-RunnerAbsolutePath([string]$Path) {
    if (!$Path) { return $false }
    if ($PSVersionTable.PSVersion.Major -ge 6) { return [IO.Path]::IsPathFullyQualified($Path) }
    return $Path -match '^(?:[A-Za-z]:[\\/]|[\\/]{2}[^\\/]+[\\/][^\\/]+(?:[\\/]|$))'
}

function Get-RunnerRelativePath([string]$Root, [string]$Path) {
    if ($PSVersionTable.PSVersion.Major -ge 6) { return [IO.Path]::GetRelativePath($Root, $Path) }
    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    $fullPath = [IO.Path]::GetFullPath($Path)
    if (![IO.Path]::GetPathRoot($fullRoot).Equals([IO.Path]::GetPathRoot($fullPath),[StringComparison]::OrdinalIgnoreCase)) { return $fullPath }
    $relative = [Uri]::UnescapeDataString(([Uri]$fullRoot).MakeRelativeUri([Uri]$fullPath).ToString()).Replace('/',[IO.Path]::DirectorySeparatorChar)
    if (!$relative) { return '.' }
    return $relative
}

function ConvertTo-RunnerHex([byte[]]$Bytes) {
    return [BitConverter]::ToString($Bytes).Replace('-','')
}

function Get-RunnerSha256([byte[]]$Bytes) {
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return ConvertTo-RunnerHex ($algorithm.ComputeHash($Bytes)) } finally { $algorithm.Dispose() }
}

function ConvertTo-RunnerHashtable($Value) {
    if ($null -eq $Value) { return $null }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $result = @{}
        foreach ($property in $Value.PSObject.Properties) { $result[$property.Name] = ConvertTo-RunnerHashtable $property.Value }
        return $result
    }
    if ($Value -is [array]) { return ,@($Value | ForEach-Object { ConvertTo-RunnerHashtable $_ }) }
    return $Value
}

function ConvertTo-RunnerJson {
    param([Parameter(ValueFromPipeline=$true)]$Value, [int]$Depth = 50)
    process {
        $json = ConvertTo-Json -InputObject $Value -Depth $Depth -Compress
        return [regex]::Replace($json, '[^\x00-\x7F]', { param($match) '\u{0:x4}' -f [int][char]$match.Value })
    }
}

function Get-RunnerPowerShell {
    if ($IsWindows) {
        foreach ($name in @('powershell.exe','pwsh.exe')) {
            $command = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($command) { return $command.Source }
        }
        return Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    }
    return (Get-Process -Id $PID).Path
}


$runnerPathComparison = if ($IsWindows) { [StringComparison]::OrdinalIgnoreCase } else { [StringComparison]::Ordinal }
$runnerJavaRelativePath = if ($IsWindows) { 'bin/java.exe' } else { 'bin/java' }
$runnerPythonRelativePath = if ($IsWindows) { 'Scripts/python.exe' } else { 'bin/python' }
$runnerHome = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
function Test-RunnerPathWithin([string]$Path, [string]$Root) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    return $fullPath.Equals($fullRoot.TrimEnd([IO.Path]::DirectorySeparatorChar), $runnerPathComparison) -or $fullPath.StartsWith($fullRoot, $runnerPathComparison)
}
