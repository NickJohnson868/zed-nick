#requires -Version 7.0
param(
    [string]$Npm = (Get-Command $(if ($IsWindows) { 'npm.cmd' } else { 'npm' }) -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source,
    [string]$Python = (Get-Command $(if ($IsWindows) { 'python.exe' } else { 'python3' }) -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source,
    [string]$JavaHome,
    [string]$Go
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$root = Join-Path ([IO.Path]::GetTempPath()) ("zed runner ' task " + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory((Join-Path $root '.zed'))
$environments = @{javascript=@{runtime=$Npm};python=@{runtime=$Python}}
if ($JavaHome) {
    $environments.java = @{runtime=$JavaHome;args=@("argument ' `$literal with spaces");jvm_args=@('-Xmx64m')}
    $javaDirectory = Join-Path $root "java source ' quoted"
    [void][IO.Directory]::CreateDirectory($javaDirectory)
    Set-Content -LiteralPath (Join-Path $javaDirectory 'Main.java') -Value @'
public class Main {
    public static void main(String[] args) {
        System.out.println("JAVA_TASK_OK");
        System.out.println(args[0]);
    }
}
'@
}
if ($Go) {
    $environments.go = @{runtime=$Go}
    Set-Content -LiteralPath (Join-Path $root 'go.mod') -Value "module runner-smoke`n`ngo 1.22"
    Set-Content -LiteralPath (Join-Path $root 'main.go') -Value "package main`nimport `"fmt`"`nfunc main() { fmt.Println(`"GO_TASK_OK`") }"
}
@{ environments=$environments;entries=@() } |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $root '.zed/run-environments.json')
@{ scripts=@{dev='node probe.cjs';failure='node -e "process.exit(7)"'} } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'package.json')
Set-Content -LiteralPath (Join-Path $root 'probe.cjs') -Value "console.log('NPM_TASK_OK'); console.log(process.cwd());"
$pythonFile = "script ' `$literal.py"
Set-Content -LiteralPath (Join-Path $root $pythonFile) -Value @'
if __name__ == "__main__":
    print("PYTHON_TASK_OK")
'@
$provider = Join-Path (Split-Path $PSScriptRoot -Parent) 'run-provider.ps1'
$result = & $provider -Root $root | ConvertFrom-Json

function Invoke-GeneratedTask($Task) {
    $command = $Task.command + ' ' + ($Task.args -join ' ')
    $shell = $Task.shell.with_arguments
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $shell.program
    $start.WorkingDirectory = $Task.cwd
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in @($shell.args) + @('-Command', $command)) { $start.ArgumentList.Add($argument) }
    foreach ($variable in $Task.env.PSObject.Properties) { $start.Environment[$variable.Name] = [string]$variable.Value }
    $process = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(90000)) {
            $process.Kill($true)
            throw 'Owned smoke-test subprocess timed out'
        }
        return @{ code=$process.ExitCode; output=$stdout.GetAwaiter().GetResult()+$stderr.GetAwaiter().GetResult() }
    } finally { $process.Dispose() }
}

$npmTask = $result.entries | Where-Object id -eq 'javascript:package.json:dev'
if (!$npmTask) { throw 'npm task was not discovered' }
$npmResult = Invoke-GeneratedTask $npmTask.task
if ($npmResult.code -ne 0 -or !$npmResult.output.Contains('NPM_TASK_OK') -or !$npmResult.output.Contains($root)) {
    throw "npm task failed: $($npmResult.output)"
}
$failureTask = $result.entries | Where-Object id -eq 'javascript:package.json:failure'
$failureResult = Invoke-GeneratedTask $failureTask.task
if ($failureResult.code -ne 7) { throw "Task exit code lost: $($failureResult.code)" }
$pythonTask = $result.entries | Where-Object id -eq "python:$pythonFile"
if (!$pythonTask) { throw 'Python task was not discovered' }
$pythonResult = Invoke-GeneratedTask $pythonTask.task
if ($pythonResult.code -ne 0 -or !$pythonResult.output.Contains('PYTHON_TASK_OK')) {
    throw "Quoted Python path failed: $($pythonResult.output)"
}
& $Python -m venv --without-pip (Join-Path $root '.venv')
if ($LASTEXITCODE -ne 0) { throw 'Fixture virtual environment creation failed' }
Add-Content -LiteralPath (Join-Path $root $pythonFile) -Value "import os, sys; print('VENV_OK=' + str(os.path.samefile(sys.prefix, os.path.join(os.path.dirname(__file__), '.venv'))))"
$venvEntries = & $provider -Root $root | ConvertFrom-Json
$venvTask = $venvEntries.entries | Where-Object id -eq "python:$pythonFile"
$venvResult = Invoke-GeneratedTask $venvTask.task
if ($venvResult.code -ne 0 -or !$venvResult.output.Contains('VENV_OK=True')) {
    throw "Native virtual environment path failed: $($venvResult.output)"
}
if ($JavaHome) {
    $javaTask = $result.entries | Where-Object language -eq 'java'
    if (!$javaTask) { throw 'Java task was not discovered' }
    $javaResult = Invoke-GeneratedTask $javaTask.task
    if ($javaResult.code -ne 0 -or !$javaResult.output.Contains('JAVA_TASK_OK') -or !$javaResult.output.Contains("argument ' `$literal with spaces")) {
        throw "Java task failed: $($javaResult.output)"
    }
}
if ($Go) {
    $goTask = $result.entries | Where-Object language -eq 'go'
    if (!$goTask) { throw 'Go task was not discovered' }
    $goResult = Invoke-GeneratedTask $goTask.task
    if ($goResult.code -ne 0 -or !$goResult.output.Contains('GO_TASK_OK')) { throw "Go task failed: $($goResult.output)" }
}
Write-Host 'Passed: npm runtime path, working directory with spaces/apostrophe, literal script path, and nonzero exit code.'
Write-Host 'Passed: Python global interpreter and native virtual environment layout.'
if ($JavaHome) { Write-Host 'Passed: Java source launch and literal application argument through the generated PowerShell task.' }
if ($Go) { Write-Host 'Passed: Go module launch through the generated task.' }
Write-Host "Temporary fixture: $root"

exit 0
