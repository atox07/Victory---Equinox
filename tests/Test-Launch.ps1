[CmdletBinding()]
param([Parameter(Mandatory)][string]$PackageDir, [Parameter(Mandatory)][string]$ProbeExe)
$ErrorActionPreference = 'Stop'
$package = (Get-Item -LiteralPath $PackageDir).FullName
$probe = (Get-Item -LiteralPath $ProbeExe).FullName
$temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$name = 'equinox synthetic ' + [Guid]::NewGuid().ToString('N')
$root = [IO.Directory]::CreateDirectory((Join-Path $temp $name)).FullName
$oldLocal = $env:LOCALAPPDATA
$oldPrivate = $env:VICTORY_SYNTHETIC_PRIVATE
try {
    $copy = [IO.Directory]::CreateDirectory((Join-Path $root 'tester package')).FullName
    # Include only the scripts and profiles the launcher actually uses.
    foreach ($f in @('Launch.ps1','Diagnose.ps1','profiles','tools')) {
        Copy-Item -LiteralPath (Join-Path $package $f) -Destination $copy -Recurse
    }
    $runtime = Join-Path $copy 'synthetic runtime.exe'
    Copy-Item -LiteralPath $probe -Destination $runtime
    $titleDir = [IO.Directory]::CreateDirectory((Join-Path $root 'title with spaces')).FullName
    $title = Join-Path $titleDir 'synthetic title.exe'
    [IO.File]::WriteAllText($title,'synthetic placeholder')
    $env:LOCALAPPDATA = [IO.Directory]::CreateDirectory((Join-Path $root 'local data')).FullName
    $env:VICTORY_SYNTHETIC_PRIVATE = 'must not be inherited'
    & (Join-Path $copy 'Launch.ps1') -Title $title -Runtime $runtime -WarmMode Skip -Profile MID-RANGE -Ram 16 -InPlace Off
    if ($LASTEXITCODE -ne 0) { throw "Synthetic launch exit $LASTEXITCODE" }
    $sessions = @(Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'VIctory-Equinox/sessions') -Directory)
    if ($sessions.Count -ne 1) { throw 'Expected one isolated session.' }
    $session = $sessions[0].FullName
    if ((Get-Content -LiteralPath (Join-Path $session 'run.stdout.log') -Raw) -notmatch 'EQUINOX_SYNTHETIC_ARGV_OK') { throw 'Probe did not confirm argv.' }
    $meta = Get-Content -LiteralPath (Join-Path $session 'session.json') -Raw | ConvertFrom-Json
    if ($meta.exit_code -ne 0 -or $meta.profile -ne 'MID-RANGE') { throw 'Incorrect session metadata.' }
    $archives = @(Get-ChildItem -LiteralPath $session -Filter '*.zip')
    if ($archives.Count -ne 1) { throw 'Missing diagnostic ZIP.' }
    $out = Join-Path $root 'diagnostic result'
    Expand-Archive -LiteralPath $archives[0].FullName -DestinationPath $out
    $stats = Get-Content -LiteralPath (Join-Path $out 'summary.json') -Raw | ConvertFrom-Json
    if ($stats.frame_intervals -ne 2 -or $stats.mean_fps -ne 100) { throw 'Unexpected probe statistics.' }
    Write-Host 'PASS: packaged launcher, quoted paths, process environment isolation, diagnostic round trip.'
} finally {
    $env:LOCALAPPDATA = $oldLocal
    $env:VICTORY_SYNTHETIC_PRIVATE = $oldPrivate
    if ([IO.Path]::GetDirectoryName($root).TrimEnd('\','/') -eq $temp.TrimEnd('\','/') -and [IO.Path]::GetFileName($root) -eq $name) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
