[CmdletBinding()]
param(
    [string]$Title,
    [ValidateSet('MID-RANGE','BASELINE')][string]$Profile = 'MID-RANGE',
    [ValidateSet('Auto','16','32')][string]$Ram = 'Auto',
    [ValidateSet('series-s','series-x')][string]$Console,
    [ValidateSet('1920x1080','1600x900','1280x720','fullscreen')][string]$Window,
    [ValidateSet('BuiltIn','Title')][string]$Upscale,
    [ValidateSet('Auto','Off','On')][string]$InPlace = 'Auto',
    [ValidateSet('Auto','Force','Skip','Cold')][string]$WarmMode = 'Auto',
    [string]$ShaderSource, [string]$BcpackTables,
    [ValidateSet('Thin','GroupedClamp','StrictViews','LegacyMul','RectFormats','NaNGuard','AdaptivePools','SmartCopyBack','WideRefresh','MetaCache','HeapDiff')]
    [string[]]$Disable = @(),
    [ValidateRange(0,64)][int]$UploadGapKiB = 0,
    [switch]$Diagnostics, [switch]$GpuTiming, [switch]$ShowSettings,
    [string]$Runtime
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'tools\Settings.ps1')
$spec = Get-Content -LiteralPath (Join-Path $PSScriptRoot "profiles\$Profile.json") -Raw | ConvertFrom-Json
if (!$Window) { $Window = $spec.window }
if (!$Console) { $Console = $spec.console }
if (!$Upscale) { $Upscale = $spec.upscale }
$logical = [Environment]::ProcessorCount
$physicalBytes = $null
try { $physicalBytes = (Get-CimInstance Win32_ComputerSystem -ErrorAction Stop).TotalPhysicalMemory } catch {}
$tier = if ($Ram -ne 'Auto') { [int]$Ram } elseif ($physicalBytes -ge 24GB) { 32 } else { 16 }
$link = if ($Profile -eq 'MID-RANGE') { Get-NvidiaLink }
$settings = Get-RunSettings $spec $logical $tier $link $InPlace
if ($Profile -eq 'BASELINE' -and $InPlace -eq 'On') { throw 'Use MID-RANGE for the in-place experiment.' }
$settings.VICTORY_WINDOW_SIZE = $Window
if ($Console -eq 'series-x') { $settings.VICTORY_RESOLUTION = '1920x1080' }
if ($Upscale -eq 'Title') { $settings.VICTORY_UPSCALE = 'title' }
$settings.VICTORY_UM_COALESCE_KB = [string]$UploadGapKiB
$toggle = @{Thin='THIN';GroupedClamp='GROUPED_MESH_CLAMP';StrictViews='STRICT_TEXTURE_VIEWS';LegacyMul='LEGACY_MUL_FTZ';RectFormats='RECT_NORMALIZED';NaNGuard='NAN_GUARD';AdaptivePools='ADAPTIVE_POOLS';SmartCopyBack='SMART_COPY_BACK';WideRefresh='WIDE_REFRESH';MetaCache='SHADER_META_CACHE';HeapDiff='HEAP_SYNC_DIFF'}
foreach ($feature in $Disable) { $settings['VICTORY_' + $toggle[$feature]] = '0' }
if ($GpuTiming) { $Diagnostics = $true; $settings.VICTORY_GPU_TIMING = '1' }
if ($Diagnostics) { $settings.VICTORY_D3D12_DRED = '1' }
$meta = [ordered]@{
    format = 1; version = '0.1.0-equinox.1'; profile = $Profile; console = $Console; window = $Window; upscale = $Upscale
    ram_tier_gib = $tier; physical_ram_bytes = $physicalBytes; logical_processors = $logical
    nominal_pcie_gbs = $link; warm_mode = $WarmMode; diagnostics = [bool]$Diagnostics
    gpu_timing = [bool]$GpuTiming; settings = ([ordered]@{} + $settings)
}
if ($ShowSettings) { $meta | ConvertTo-Json -Depth 5; return }
if (!$Title -or !(Test-Path -LiteralPath $Title -PathType Leaf)) { throw 'Supply a local, lawfully obtained title executable with -Title.' }
$titlePath = (Get-Item -LiteralPath $Title).FullName
if ($Runtime) { $exe = (Get-Item -LiteralPath $Runtime -ErrorAction Stop).FullName }
else {
    $name = if ($Diagnostics) { 'victory-diagnostics.exe' } else { 'victory.exe' }
    $build = if ($Diagnostics) { 'diag' } else { 'release' }
    $exe = @((Join-Path $PSScriptRoot $name),(Join-Path $PSScriptRoot "target\x86_64-pc-windows-msvc\$build\victory.exe"),(Join-Path $PSScriptRoot "target\$build\victory.exe")) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (!$exe) { throw "Missing $name. Use the Windows x64 tester ZIP or build the source as described in README.md." }
}
if ($BcpackTables) { $settings.VICTORY_BCPACK_TABLES = (Get-Item -LiteralPath $BcpackTables -ErrorAction Stop).FullName }
elseif ($env:VICTORY_BCPACK_TABLES) { $settings.VICTORY_BCPACK_TABLES = (Get-Item -LiteralPath $env:VICTORY_BCPACK_TABLES -ErrorAction Stop).FullName }
if ($settings.Contains('VICTORY_BCPACK_TABLES') -and (Get-Item -LiteralPath $settings.VICTORY_BCPACK_TABLES).Length -ne 77982) { throw 'The selected BCPack table file has the wrong size.' }
$settings.VICTORY_GAME_DIR = [IO.Path]::GetDirectoryName($titlePath)
$mutex = [Threading.Mutex]::new($false, ('Local\VIctory-MidRange-' + (Get-LocalHash $titlePath.ToUpperInvariant())))
try { $owned = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $owned = $true }
if (!$owned) { $mutex.Dispose(); throw 'This title already has a VIctory Equinox session.' }
try {
$work = Join-Path $env:LOCALAPPDATA 'VIctory-Equinox'
$tag = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)
$session = Join-Path $work "sessions\$tag"
[void][IO.Directory]::CreateDirectory($session)
$meta.runtime_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash
$meta.cpu = $null; $meta.display_adapters = @()
try { $meta.cpu = @(Get-CimInstance Win32_Processor -ErrorAction Stop)[0].Name } catch {}
try { $meta.display_adapters = @(Get-CimInstance Win32_VideoController -ErrorAction Stop | ForEach-Object { [pscustomobject]@{model=$_.Name; driver=$_.DriverVersion} }) } catch {}
$cacheIdentity = ($meta.settings | ConvertTo-Json -Compress) + $Console + $Upscale + $meta.runtime_sha256 + ($meta.display_adapters | ConvertTo-Json -Compress) + $titlePath
$cacheKey = (Get-LocalHash $cacheIdentity).Substring(0,24)
$settings.VICTORY_CACHE_DIR = if ($WarmMode -eq 'Cold') { Join-Path $work "cache\cold-$tag" } else { Join-Path $work "cache\$Profile-$cacheKey" }
$settings.VICTORY_DATA_DIR = Join-Path $work ('data\' + (Get-LocalHash $titlePath).Substring(0,16))
[void][IO.Directory]::CreateDirectory($settings.VICTORY_CACHE_DIR)
[void][IO.Directory]::CreateDirectory($settings.VICTORY_DATA_DIR)
$settings.VICTORY_FRAMETIME_DUMP = Join-Path $session 'intervals-local.csv'
$settings.VICTORY_PRESENT_CSV = Join-Path $session 'frames.csv'
$settings.VICTORY_LOG = 'info'
if ($Profile -eq 'BASELINE') {
    $ambient = Join-Path $session 'ambient-local.txt'
    [IO.File]::WriteAllText($ambient, '0.36 0.05 0.4 1.0 0.6 0.1 0.45 0.5')
    $settings.VICTORY_AMBIENT_FILE = $ambient
}

function Start-LocalRuntime([string[]]$Arguments, [string]$Phase, [switch]$Sample) {
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.WorkingDirectory = $session
    $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    # Windows file names cannot contain quotes. Escape trailing slashes for CommandLineToArgvW.
    $psi.Arguments = (($Arguments | ForEach-Object { '"' + ($_ -replace '(\\+)$','$1$1') + '"' }) -join ' ')
    foreach ($key in @($psi.EnvironmentVariables.Keys)) { if ($key -like 'VICTORY_*') { $psi.EnvironmentVariables.Remove($key) } }
    foreach ($key in $settings.Keys) { $psi.EnvironmentVariables[$key] = [string]$settings[$key] }
    $out = [IO.File]::Create((Join-Path $session "$Phase.stdout.log"))
    $err = [IO.File]::Create((Join-Path $session "$Phase.stderr.log"))
    $proc = New-Object Diagnostics.Process
    try {
        $proc.StartInfo = $psi
        if (!$proc.Start()) { throw "Cannot start $Phase." }
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $stdout = $proc.StandardOutput.BaseStream.CopyToAsync($out)
        $stderr = $proc.StandardError.BaseStream.CopyToAsync($err)
        if ($Sample) {
            while (!$proc.HasExited) {
                Get-SystemSample $proc $clock.Elapsed.TotalSeconds | Export-Csv -NoTypeInformation -Append -LiteralPath (Join-Path $session 'system.csv')
                if ($proc.WaitForExit(5000)) { break }
            }
        }
        $proc.WaitForExit()
        [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($stdout,$stderr))
        return $proc.ExitCode
    } finally { $proc.Dispose(); $out.Dispose(); $err.Dispose() }
}

$meta.prewarm = 'skipped'
$meta.warm_translated = $null; $meta.warm_refused = $null; $meta.warm_pipelines = $null; $meta.warm_pipeline_refused = $null
if ($WarmMode -in @('Auto','Force')) {
    $source = if ($ShaderSource) { (Get-Item -LiteralPath $ShaderSource).FullName } else { Join-Path $settings.VICTORY_GAME_DIR 'shaders.rpf' }
    if (Test-Path -LiteralPath $source) {
        $item = Get-Item -LiteralPath $source
        $items = if ($item.PSIsContainer) { Get-ChildItem -LiteralPath $source -File -Recurse | Sort-Object FullName } else { $item }
        $signature = Get-LocalHash (($items | ForEach-Object { "$($_.FullName)|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)" }) -join [Environment]::NewLine)
        $stamp = Join-Path $settings.VICTORY_CACHE_DIR 'warm-v1.json'
        $previous = $null
        if (Test-Path -LiteralPath $stamp) { try { $previous = Get-Content -LiteralPath $stamp -Raw | ConvertFrom-Json } catch {} }
        if ($WarmMode -eq 'Force' -or $previous.signature -ne $signature -or $previous.refused -or $previous.pipeline_refused) {
            Write-Host 'Warming local shaders...'
            $code = Start-LocalRuntime -Arguments @('warm',$source,'--jobs',$settings.VICTORY_PIPELINE_WORKERS) -Phase warm
            $line = Get-Content -LiteralPath (Join-Path $session 'warm.stdout.log') -Tail 10 | Select-String '^warm\s+(\d+) translated, (\d+) refused; (\d+) compute pipelines, (\d+) refused' | Select-Object -Last 1
            if ($code -eq 0 -and $line) {
                $groups = $line.Matches[0].Groups
                $meta.warm_translated = [int]$groups[1].Value; $meta.warm_refused = [int]$groups[2].Value
                $meta.warm_pipelines = [int]$groups[3].Value; $meta.warm_pipeline_refused = [int]$groups[4].Value
                $meta.prewarm = if (!$meta.warm_translated) { 'no-shaders' } elseif ($meta.warm_refused -or $meta.warm_pipeline_refused) { 'partial' } else { 'completed' }
                if ($meta.warm_translated) {
                    [ordered]@{signature=$signature; translated=$meta.warm_translated; refused=$meta.warm_refused; pipelines=$meta.warm_pipelines; pipeline_refused=$meta.warm_pipeline_refused} | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath $stamp
                }
            } else { $meta.prewarm = 'failed'; Write-Warning "Warm-up produced no valid summary (exit $code). See the session logs." }
        } else {
            $meta.prewarm = 'cached'
            $meta.warm_translated = $previous.translated; $meta.warm_refused = $previous.refused
            $meta.warm_pipelines = $previous.pipelines; $meta.warm_pipeline_refused = $previous.pipeline_refused
        }
    } elseif ($WarmMode -eq 'Force') { throw 'Specify -ShaderSource with a local shader input for forced warm-up.' }
    else { $meta.prewarm = 'no-input' }
}
$sessionFile = Join-Path $session 'session.json'
$meta.exit_code = $null
$meta | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $sessionFile -Encoding UTF8
Write-Host "VIctory Equinox | $Profile | $Console | RAM tier $tier GiB"
Write-Host "Session: $session"
$exitCode = Start-LocalRuntime -Arguments @('run',$titlePath,'--execute','--console',$Console) -Phase run -Sample
$meta.exit_code = $exitCode
$meta | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $sessionFile -Encoding UTF8
& (Join-Path $PSScriptRoot 'Diagnose.ps1') -Session $session
} finally {
    $mutex.ReleaseMutex()
    $mutex.Dispose()
}
exit $exitCode
