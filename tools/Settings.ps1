# Process-local profile calculation. The launcher uses the result for both warm-up and play.
function Get-NvidiaLink {
    try {
        $adapters = @(Get-PnpDevice -PresentOnly -Class Display -ErrorAction Stop | Where-Object { $_.InstanceId -like 'PCI\*' })
        if ($adapters.Count -ne 1 -or $adapters[0].InstanceId -notlike 'PCI\VEN_10DE*') { return }
        $id = $adapters[0].InstanceId
        $gen = [int](Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_PciDevice_CurrentLinkSpeed -ErrorAction Stop).Data
        $lanes = [int](Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_PciDevice_CurrentLinkWidth -ErrorAction Stop).Data
        if ($gen -in 1..5 -and $lanes -in @(1,2,4,8,12,16,32)) {
            # Nominal decimal GB/s per direction, using the same rounded link classes as Neptune FPS.
            return (@(0,0.25,0.5,1,2,4)[$gen] * $lanes)
        }
    } catch { return }
}

function Get-RunSettings {
    param($Spec, [int]$Logical, [int]$MemoryTier, $Link, [string]$InPlace)
    $set = [ordered]@{}
    foreach ($item in $Spec.settings.PSObject.Properties) { $set[$item.Name] = [string]$item.Value }
    if ($Spec.name -eq 'BASELINE') {
        $thin = [Math]::Max(4,[Math]::Min(8,[int][Math]::Floor($Logical/2)))
        $turbo = [Math]::Max(3,[Math]::Min(6,$thin-1))
        $wide = $thin
    } else {
        $thin = [Math]::Max(1,[Math]::Min(6,[int][Math]::Floor($Logical/3)))
        $turbo = [Math]::Max(1,[Math]::Min(4,[int][Math]::Floor($Logical/4)))
        $wide = [Math]::Max(2,[Math]::Min(4,$turbo))
        $set.VICTORY_PIPELINE_WORKERS = [string][Math]::Max(1,[Math]::Min(3,[int][Math]::Floor($Logical/6)))
        foreach ($item in $Spec.memoryTiers."$MemoryTier".PSObject.Properties) { $set[$item.Name] = [string]$item.Value }
        $inPlaceOn = $InPlace -eq 'On' -or ($InPlace -eq 'Auto' -and $null -ne $Link -and $Link -ge 16)
        $set.VICTORY_INPLACE = [string][int]$inPlaceOn
    }
    $set.VICTORY_THIN_THREADS = [string]$thin
    $set.VICTORY_TURBO_THREADS = [string]$turbo
    $set.VICTORY_WIDE_THREADS = [string]$wide
    return $set
}

function Get-LocalHash([string]$Value) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Value)))).Replace('-','').ToLowerInvariant()
    } finally { $sha.Dispose() }
}

function Get-SystemSample($Process, [double]$Seconds) {
    $sample = [ordered]@{
        elapsed_s = [Math]::Round($Seconds,3); utc_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); working_set_bytes = $null; process_commit_bytes = $null
        available_mib = $null; committed_bytes = $null; commit_limit_bytes = $null
        pages_in_per_s = $null; pages_out_per_s = $null; page_reads_per_s = $null
    }
    try {
        $Process.Refresh()
        $sample.working_set_bytes = $Process.WorkingSet64
        $sample.process_commit_bytes = $Process.PrivateMemorySize64
    } catch {}
    try {
        $m = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory -ErrorAction Stop
        $sample.available_mib = $m.AvailableMBytes
        $sample.committed_bytes = $m.CommittedBytes
        $sample.commit_limit_bytes = $m.CommitLimit
        $sample.pages_in_per_s = $m.PagesInputPersec
        $sample.pages_out_per_s = $m.PagesOutputPersec
        $sample.page_reads_per_s = $m.PageReadsPersec
    } catch {}
    return [pscustomobject]$sample
}
