[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Session,
    [ValidateRange(0,86400)][double]$StartSecond = 0,
    [ValidateRange(0,86400)][double]$DurationSeconds = 0
)
$ErrorActionPreference = 'Stop'
$root = (Get-Item -LiteralPath $Session -ErrorAction Stop).FullName
$sessionData = Get-Content -LiteralPath (Join-Path $root 'session.json') -Raw | ConvertFrom-Json
$inv = [Globalization.CultureInfo]::InvariantCulture
$startUs = $StartSecond * 1e6
$stopUs = if ($DurationSeconds -gt 0) { $startUs + $DurationSeconds * 1e6 } else { [double]::PositiveInfinity }
$durations = [Collections.Generic.List[long]]::new()
$name = 'VIctory-Equinox-diagnostics-' + [Guid]::NewGuid().ToString('N').Substring(0,12)
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$stage = [IO.Path]::GetFullPath((Join-Path $tempRoot $name))
[void][IO.Directory]::CreateDirectory($stage)

function Write-NumericTable([string]$InputName,[string]$OutputName,[string[]]$Columns) {
    $source = Join-Path $root $InputName
    $writer = [IO.StreamWriter]::new((Join-Path $stage $OutputName),$false,[Text.UTF8Encoding]::new($false))
    try {
        $writer.WriteLine(($Columns -join ','))
        if (!(Test-Path -LiteralPath $source -PathType Leaf)) { return }
        Get-Content -LiteralPath $source | Where-Object { $_ -and !$_.StartsWith('#') } | ConvertFrom-Csv | ForEach-Object {
            $row = $_
            $values = foreach ($column in $Columns) {
                $value = [string]$row.$column
                if ($value -match '^-?\d{1,20}(?:\.\d{1,12})?$') { $value } else { '' }
            }
            if ($InputName -eq 'frames.csv') {
                if ($values[0] -notmatch '^\d{1,16}$' -or $values[1] -notmatch '^\d{1,16}$') { return }
                $at = [long]$values[0]; $us = [long]$values[1]
                if ($us -le 0 -or $at -lt $us -or $at - $us -lt $startUs -or $at -gt $stopUs) { return }
                $durations.Add($us)
            }
            if ($InputName -eq 'intervals-local.csv' -and $values[1] -match '^\d+$') {
                $at = [double]::Parse($values[1],$inv) * 1000
                if ($at -lt $startUs -or $at -gt $stopUs) { return }
            }
            $writer.WriteLine(($values -join ','))
        }
    } finally { $writer.Dispose() }
}

try {
    Write-NumericTable 'frames.csv' 'frames.csv' @('elapsed_us','frametime_us')
    Write-NumericTable 'intervals-local.csv' 'intervals.csv' @(
        't_ms','present_elapsed_ms','frame','frames','fps','ms_per_frame','cpu_prepare_ms','gpu_ms','gpu_command_ms','gpu_pct',
        'copies_ms','copies_mb','textures_ms','textures_mb','copyback_ms','copyback_mb','draws','dropped','invalid_indirect',
        'making','indirect','heap_ms','texture_refresh_ms','card_sent_mb','card_ranges','card_fetched_mb','card_waits','card_wait_ms',
        'dxgi_budget_bytes','dxgi_local_bytes','dxgi_shared_budget_bytes','dxgi_shared_bytes','ram_mb','cpu_pct'
    )
    Write-NumericTable 'system.csv' 'system.csv' @('elapsed_s','utc_ms','working_set_bytes','process_commit_bytes','available_mib','committed_bytes','commit_limit_bytes','pages_in_per_s','pages_out_per_s','page_reads_per_s')
    $durations.Sort()
    $count = $durations.Count
    $summary = [ordered]@{
        source = 'successful DXGI present intervals'; start_second = $StartSecond; duration_limit_s = $DurationSeconds
        frame_intervals = $count; measured_s = $null; mean_fps = $null; one_percent_low_fps = $null
        mean_ms = $null; p95_ms = $null; p99_ms = $null; slowest_one_percent_count = 0
    }
    if ($count -gt 0) {
        [double]$total = 0
        foreach ($n in $durations) { $total += $n }
        $worst = [int][Math]::Ceiling($count * 0.01)
        [double]$slow = 0
        for ($i = $count - $worst; $i -lt $count; $i++) { $slow += $durations[$i] }
        $summary.measured_s = [Math]::Round($total/1e6,6)
        $summary.mean_fps = [Math]::Round($count*1e6/$total,3)
        $summary.one_percent_low_fps = [Math]::Round($worst*1e6/$slow,3)
        $summary.mean_ms = [Math]::Round($total/($count*1000),3)
        $summary.p95_ms = $durations[[int][Math]::Ceiling(0.95*$count)-1]/1000.0
        $summary.p99_ms = $durations[[int][Math]::Ceiling(0.99*$count)-1]/1000.0
        $summary.slowest_one_percent_count = $worst
    }
    $events = [ordered]@{ shader_error_lines=0; d3d_error_lines=0; device_removed_lines=0; out_of_memory_lines=0; crash_lines=0; log_files_read=0 }
    $actualWorkers = [ordered]@{}
    foreach ($log in @('warm.stderr.log','run.stderr.log','run.stdout.log')) {
        $file = Join-Path $root $log
        if (!(Test-Path -LiteralPath $file -PathType Leaf)) { continue }
        $events.log_files_read++
        foreach ($line in [IO.File]::ReadLines($file)) {
            $clean = $line -replace '\x1B\[[0-9;]*[a-zA-Z]',''
            if ($clean -match '(shader|DXIL|translat).{0,120}(error|fail|refus|unsupported)|(failed|unable).{0,40}(shader|DXIL|translat)') { $events.shader_error_lines++ }
            if ($clean -match 'D3D12.{0,100}(error|fail)|DXGI_ERROR|0x887a00[0-9a-f]{2}') { $events.d3d_error_lines++ }
            if ($clean -match 'device (removed|hung|reset)|DXGI_ERROR_DEVICE_(REMOVED|HUNG|RESET)|0x887a000[567]') { $events.device_removed_lines++ }
            if ($clean -match 'out of memory|0x8007000e|allocation failed') { $events.out_of_memory_lines++ }
            if ($clean -match 'panicked at|unhandled exception|fatal error|access violation') { $events.crash_lines++ }
            if ($clean -match 'Neptune HyperEngine worker pool active' -and $clean -match 'threads[=: ]+(\d{1,2})') { $actualWorkers.turbo = [int]$Matches[1] }
            if ($clean -match 'list calls are written down and translated' -and $clean -match 'threads[=: ]+(\d{1,2})') { $actualWorkers.thin = [int]$Matches[1] }
        }
    }
    # Public settings contain only switches emitted by these two profiles, never filenames or raw logs.
    $allowed = @{}
    foreach ($profile in @('BASELINE','MID-RANGE')) {
        $spec = Get-Content -LiteralPath (Join-Path $PSScriptRoot "profiles\$profile.json") -Raw | ConvertFrom-Json
        foreach ($k in $spec.settings.PSObject.Properties.Name) { $allowed[$k]=$true }
        foreach ($tier in $spec.memoryTiers.PSObject.Properties) { foreach ($k in $tier.Value.PSObject.Properties.Name) { $allowed[$k]=$true } }
    }
    foreach ($k in @('THIN_THREADS','WIDE_THREADS','TURBO_THREADS','PIPELINE_WORKERS','GPU_TIMING','D3D12_DRED','RESOLUTION','WINDOW_SIZE','UPSCALE')) { $allowed["VICTORY_$k"]=$true }
    $export = [ordered]@{}
    foreach ($field in @('format','version','runtime_sha256','profile','console','window','upscale','ram_tier_gib','physical_ram_bytes','logical_processors','nominal_pcie_gbs','warm_mode','prewarm','warm_translated','warm_refused','warm_pipelines','warm_pipeline_refused','diagnostics','gpu_timing','exit_code','cpu')) { $export[$field]=$sessionData.$field }
    $export.display_adapters = @($sessionData.display_adapters | Where-Object { $_ } | Select-Object model,driver)
    $export.settings = [ordered]@{}
    foreach ($item in $sessionData.settings.PSObject.Properties) {
        if ($allowed.ContainsKey($item.Name) -and [string]$item.Value -match '^[A-Za-z0-9.,_-]{1,40}$') { $export.settings[$item.Name]=$item.Value }
    }
    $export.workers_reported_by_runtime = $actualWorkers
    foreach ($pair in @(@('configuration.json',$export),@('summary.json',$summary),@('events.json',$events))) {
        $pair[1] | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $stage $pair[0]) -Encoding UTF8
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = Join-Path $root "$name.zip"
    [IO.Compression.ZipFile]::CreateFromDirectory($stage,$zip)
    if ($count) { Write-Host ("{0} intervals | mean {1} FPS | 1% low {2} FPS | P95 {3} ms | P99 {4} ms" -f $count,$summary.mean_fps,$summary.one_percent_low_fps,$summary.p95_ms,$summary.p99_ms) }
    else { Write-Host 'No valid presentation intervals in this window. Performance metrics are unavailable.' }
    Write-Host "Diagnostic ZIP: $zip"
} finally {
    if ([IO.Path]::GetDirectoryName($stage).TrimEnd('\','/') -eq $tempRoot.TrimEnd('\','/') -and [IO.Path]::GetFileName($stage) -eq $name) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
}
