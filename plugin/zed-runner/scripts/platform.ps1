$runnerPathComparison = if ($IsWindows) { [StringComparison]::OrdinalIgnoreCase } else { [StringComparison]::Ordinal }
$runnerJavaRelativePath = if ($IsWindows) { 'bin/java.exe' } else { 'bin/java' }
$runnerPythonRelativePath = if ($IsWindows) { 'Scripts/python.exe' } else { 'bin/python' }
$runnerHome = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
function Test-RunnerPathWithin([string]$Path, [string]$Root) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    return $fullPath.Equals($fullRoot.TrimEnd([IO.Path]::DirectorySeparatorChar), $runnerPathComparison) -or $fullPath.StartsWith($fullRoot, $runnerPathComparison)
}
