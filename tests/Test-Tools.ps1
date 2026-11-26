# Synthetic regression tests for the launcher and the diagnostic boundary.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
. (Join-Path $repo 'tools/Settings.ps1')
function Assert-Value($Actual,$Expected,[string]$Label) {
    if ($Actual -ne $Expected) { throw "$Label expected [$Expected], got [$Actual]." }
}
$mid = Get-Content -LiteralPath (Join-Path $repo 'profiles/MID-RANGE.json') -Raw | ConvertFrom-Json
$ref = Get-Content -LiteralPath (Join-Path $repo 'profiles/BASELINE.json') -Raw | ConvertFrom-Json
$low = Get-RunSettings $mid 12 16 8 Auto
Assert-Value $low.VICTORY_INPLACE '0' 'PCIe 3 x8'
Assert-Value $low.VICTORY_SMART_COPY_BACK '0' 'full readback'
Assert-Value $low.VICTORY_INDIRECT_RAW '0' 'checked indirect'
Assert-Value $low.VICTORY_THIN_THREADS '4' '12-thread thin pool'
Assert-Value $low.VICTORY_TURBO_THREADS '3' '12-thread turbo pool'
Assert-Value $low.VICTORY_OCEAN_READBACK '1' 'water readback'
$high = Get-RunSettings $mid 16 32 32 Auto
Assert-Value $high.VICTORY_INPLACE '1' 'fast-link windows'
if ([int]$low.VICTORY_STAGE_POOL_MB -ge [int]$high.VICTORY_STAGE_POOL_MB) { throw 'RAM tier not reflected in staging.' }
Assert-Value (Get-RunSettings $mid 16 32 32 Off).VICTORY_INPLACE '0' 'in-place override'
Assert-Value (Get-RunSettings $mid 16 32 $null Auto).VICTORY_INPLACE '0' 'unknown link'
Assert-Value (Get-RunSettings $ref 12 16 32 Auto).VICTORY_TURBO_THREADS '5' 'reference workers'

$name = 'victory-synthetic-' + [guid]::NewGuid().ToString('N')
$temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$session = [IO.Directory]::CreateDirectory((Join-Path $temp $name)).FullName
try {
    [pscustomobject]@{profile='MID-RANGE';version='test';settings=@{VICTORY_GAME_DIR='PRIVATE_TEST_MARKER';VICTORY_THIN_THREADS='4'};exit_code=0} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $session 'session.json')
    $csv = [Collections.Generic.List[string]]::new()
    $csv.Add('elapsed_us,frametime_us')
    for ($i=1;$i -le 99;$i++) { $csv.Add(('{0},10000' -f ($i*10000))) }
    $csv.Add('1100000,110000')
    $csv.Add('1200000,PRIVATE_TEST_MARKER')
    [IO.File]::WriteAllLines((Join-Path $session 'frames.csv'),$csv)
    [IO.File]::WriteAllLines((Join-Path $session 'intervals-local.csv'),@('# PRIVATE_TEST_MARKER','t_ms,present_elapsed_ms,frame,frames,fps,gpu_ms','1234567,1000,99,99,99,4.2'))
    [IO.File]::WriteAllLines((Join-Path $session 'run.stderr.log'),@('shader translation failed PRIVATE_TEST_MARKER','D3D12 present failed hr=0x887a0005','Neptune HyperEngine worker pool active threads=3'))
    [IO.File]::WriteAllText((Join-Path $session 'crash.dump'),'PRIVATE_TEST_MARKER')
    & (Join-Path $repo 'Diagnose.ps1') -Session $session
    $archive = @(Get-ChildItem -LiteralPath $session -Filter '*.zip')[0].FullName
    $out = Join-Path $session 'extracted'
    Expand-Archive -LiteralPath $archive -DestinationPath $out
    $stats = Get-Content -LiteralPath (Join-Path $out 'summary.json') -Raw | ConvertFrom-Json
    Assert-Value $stats.frame_intervals 100 'valid intervals'
    Assert-Value $stats.mean_fps 90.909 'weighted mean FPS'
    Assert-Value $stats.one_percent_low_fps 9.091 'slowest one percent'
    Assert-Value $stats.p95_ms 10 'nearest-rank P95'
    Assert-Value $stats.p99_ms 10 'nearest-rank P99'
    Assert-Value ((Get-ChildItem -LiteralPath $out -File).Count) 6 'diagnostic allowlist'
    foreach ($file in Get-ChildItem -LiteralPath $out -File) {
        if ((Get-Content -LiteralPath $file.FullName -Raw) -match 'PRIVATE_TEST_MARKER') { throw 'Private input in exported archive.' }
    }
    $events = Get-Content -LiteralPath (Join-Path $out 'events.json') -Raw | ConvertFrom-Json
    Assert-Value $events.device_removed_lines 1 'device removal classification'
    Assert-Value $events.shader_error_lines 1 'shader error classification'
    & (Join-Path $repo 'Diagnose.ps1') -Session $session -StartSecond 0.99 -DurationSeconds 0.11
    $slice = @(Get-ChildItem -LiteralPath $session -Filter '*.zip' | Where-Object { $_.FullName -ne $archive })[0]
    $cut = Join-Path $session 'slice'
    Expand-Archive -LiteralPath $slice.FullName -DestinationPath $cut
    $s = Get-Content -LiteralPath (Join-Path $cut 'summary.json') -Raw | ConvertFrom-Json
    Assert-Value $s.frame_intervals 1 'fully contained intervals'
    Assert-Value $s.p99_ms 110 'sliced P99'
    Write-Host 'PASS: profiles, PCIe decisions, statistics, event counters and export allowlist.'
} finally {
    if ([IO.Path]::GetDirectoryName($session).TrimEnd('\','/') -eq $temp.TrimEnd('\','/') -and [IO.Path]::GetFileName($session) -eq $name) {
        Remove-Item -LiteralPath $session -Recurse -Force
    }
}
