$ignoredDirectories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($name in @('.git','.zed','.idea','.venv','venv','target','node_modules','build','dist','release','out','logs','temp','site-packages','.tox','__pycache__','.pytest_cache','.mypy_cache')) {
    [void]$ignoredDirectories.Add($name)
}

function Get-WorkspaceFiles([string]$Root, [switch]$StrictFallback) {
    $rootPath = [IO.Path]::GetFullPath($Root)
    $paths = [Collections.Generic.List[string]]::new()
    $extraDirectories = [Collections.Generic.List[string]]::new()
    $scanDirectories = [Collections.Generic.List[string]]::new()
    $ignoreConfigPath = Join-Path $rootPath '.zed/runner-ignore.json'
    $hasIgnoreConfig = Test-Path -LiteralPath $ignoreConfigPath -PathType Leaf
    if ($hasIgnoreConfig) {
        $ignoreConfig = Get-Content -LiteralPath $ignoreConfigPath -Raw | ConvertFrom-Json
        foreach ($propertyName in @('scan_directories','ignore_directories')) {
            $property = $ignoreConfig.PSObject.Properties[$propertyName]
            if (!$property -or $property.Value -isnot [array]) {
                throw "runner-ignore.json 的 $propertyName 必须是目录路径数组。"
            }
            foreach ($directory in $property.Value) {
                if ($directory -isnot [string]) { throw "runner-ignore.json 的 $propertyName 必须是目录路径字符串数组。" }
                $normalized = $directory.Replace('\','/').TrimEnd('/')
                if (!$normalized -or ($normalized -ne '.' -and
                    ([IO.Path]::IsPathFullyQualified($directory) -or
                    @($normalized.Split('/') | Where-Object { $_ -in @('.','..','') }).Count))) {
                    throw "runner-ignore.json 中的目录路径无效：$directory"
                }
                if ($propertyName -eq 'scan_directories') { $scanDirectories.Add($normalized) }
                else { $extraDirectories.Add($normalized) }
            }
        }
    }
    function Is-InScanDirectory([string]$RelativePath) {
        if (!$hasIgnoreConfig) { return $true }
        foreach ($directory in $scanDirectories) {
            if ($directory -eq '.' -or
                $RelativePath.Equals($directory,[StringComparison]::OrdinalIgnoreCase) -or
                $RelativePath.StartsWith($directory+'/',[StringComparison]::OrdinalIgnoreCase)) { return $true }
        }
        return $false
    }
    function Should-EnterDirectory([string]$RelativePath) {
        if (!$hasIgnoreConfig) { return $true }
        foreach ($directory in $scanDirectories) {
            if ($directory -eq '.' -or
                $directory.Equals($RelativePath,[StringComparison]::OrdinalIgnoreCase) -or
                $directory.StartsWith($RelativePath+'/',[StringComparison]::OrdinalIgnoreCase) -or
                $RelativePath.StartsWith($directory+'/',[StringComparison]::OrdinalIgnoreCase)) { return $true }
        }
        return $false
    }
    function Is-IgnoredDirectory([string]$RelativePath) {
        $parts = $RelativePath.Replace('\','/').Split('/')
        for ($index = 0; $index -lt $parts.Length; $index++) {
            if ($ignoredDirectories.Contains($parts[$index]) -or $parts[$index].StartsWith('.local')) { return $true }
        }
        if (!(Is-InScanDirectory $RelativePath)) { return $false }
        foreach ($directory in $extraDirectories) {
            if ($directory.Contains('/')) {
                if ($RelativePath.Equals($directory,[StringComparison]::OrdinalIgnoreCase) -or
                    $RelativePath.StartsWith($directory+'/',[StringComparison]::OrdinalIgnoreCase)) { return $true }
            } elseif ($parts -contains $directory) { return $true }
        }
        return $false
    }
    $git = Get-Command git -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($git) {
        $repositoryRoot = & $git.Source -C $rootPath rev-parse --show-toplevel 2>$null
        if ($LASTEXITCODE -eq 0 -and $repositoryRoot) {
            $listed = @(& $git.Source -c core.quotepath=false -C $rootPath ls-files --cached --others --exclude-standard 2>$null)
            if ($LASTEXITCODE -ne 0) { throw 'Git 无法列出未忽略文件，已跳过自动扫描。' }
            foreach ($path in $listed) {
                if (![string]::IsNullOrEmpty($path)) { $paths.Add($path.Replace('\','/')) }
            }
        }
    }

    if (!$repositoryRoot -or $LASTEXITCODE -ne 0) {
        $pending = [Collections.Generic.Stack[string]]::new()
        $pending.Push($rootPath)
        while ($pending.Count) {
            $directory = $pending.Pop()
            try { $items = @(Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) }
            catch {
                if ($StrictFallback) { throw }
                continue
            }
            foreach ($item in $items) {
                if ($item.PSIsContainer) {
                    $relativeDirectory = [IO.Path]::GetRelativePath($rootPath,$item.FullName).Replace('\','/')
                    if ((Should-EnterDirectory $relativeDirectory) -and
                        !(Is-IgnoredDirectory $relativeDirectory) -and
                        !$item.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
                        $pending.Push($item.FullName)
                    }
                } else {
                    $paths.Add([IO.Path]::GetRelativePath($rootPath,$item.FullName).Replace('\','/'))
                }
            }
        }
    }

    $filtered = [Collections.Generic.List[string]]::new()
    foreach ($path in $paths) {
        $parent = [IO.Path]::GetDirectoryName($path.Replace('/',[IO.Path]::DirectorySeparatorChar))
        $relativeParent = if ($parent) { $parent.Replace('\','/') } else { '' }
        if ((Is-InScanDirectory $relativeParent) -and
            (!$relativeParent -or !(Is-IgnoredDirectory $relativeParent))) { $filtered.Add($path) }
    }
    return $filtered.ToArray()
}
