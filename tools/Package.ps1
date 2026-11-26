[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ReleaseExe,
    [Parameter(Mandatory)][string]$DiagnosticExe,
    [Parameter(Mandatory)][string]$OutputDir,
    [string]$AgilityPackage
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = '1.620.2'
# Pinned by Neptune FPS in xtask/src/agility.rs. Never look up a floating version.
$expected = 'd683ff7f5726906c3f43d299f6cdd4b1cad22f8329ccd27b3492d0a1413d9904102b8d543419bec3b3545c86bf49f6a500ebf42d95c999aa0969e5d693032735'
$release = (Get-Item -LiteralPath $ReleaseExe).FullName
$diag = (Get-Item -LiteralPath $DiagnosticExe).FullName
$dir = [IO.Directory]::CreateDirectory($OutputDir).FullName
$name = 'victory-package-' + [Guid]::NewGuid().ToString('N')
$temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$scratch = [IO.Directory]::CreateDirectory((Join-Path $temp $name)).FullName
$stage = [IO.Directory]::CreateDirectory((Join-Path $scratch 'runtime')).FullName
try {
    if (!$AgilityPackage) {
        $AgilityPackage = Join-Path $scratch 'agility.nupkg'
        $url = "https://api.nuget.org/v3-flatcontainer/microsoft.direct3d.d3d12/$version/microsoft.direct3d.d3d12.$version.nupkg"
        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $AgilityPackage
    }
    if ((Get-FileHash -LiteralPath $AgilityPackage -Algorithm SHA512).Hash -ne $expected) { throw 'Agility SDK SHA-512 mismatch.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $package = [IO.Compression.ZipFile]::OpenRead((Get-Item -LiteralPath $AgilityPackage).FullName)
    try {
        $entry = $package.GetEntry('build/native/bin/x64/D3D12Core.dll')
        if (!$entry) { throw 'The pinned package contains no x64 D3D12Core.dll.' }
        $d3d = [IO.Directory]::CreateDirectory((Join-Path $stage 'D3D12')).FullName
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,(Join-Path $d3d 'D3D12Core.dll'))
        $notices = [IO.Directory]::CreateDirectory((Join-Path $stage 'licenses\Agility')).FullName
        $legal = @($package.Entries | Where-Object { $_.Name -match '(?i)license|licence|notice|redist|third.?party|\.nuspec$' -and $_.Name -match '\.(txt|md|rtf|html?|nuspec)$' })
        if (!$legal.Count) { throw 'Missing package licensing information.' }
        foreach ($entry in $legal) {
            $leaf = $entry.FullName.Replace('/','_').Replace('\','_')
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,(Join-Path $notices $leaf))
        }
    } finally { $package.Dispose() }
    Copy-Item -LiteralPath $release -Destination (Join-Path $stage 'victory.exe')
    Copy-Item -LiteralPath $diag -Destination (Join-Path $stage 'victory-diagnostics.exe')
    $public = @('Launch.ps1','Diagnose.ps1','tools/Settings.ps1','profiles/BASELINE.json','profiles/MID-RANGE.json','README.md','LICENSE','CHANGELOG.md','docs/BENCHMARK.md','docs/INTEGRATION.md','docs/CREDITS.md','docs/asset-policy.md','docs/HYPERENGINE.md','docs/UPSTREAM-README.md')
    foreach ($relative in $public) {
        $to = Join-Path $stage $relative
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($to))
        Copy-Item -LiteralPath (Join-Path $root $relative) -Destination $to
    }
    $manifest = [ordered]@{ version='0.1.0-equinox.1'; commit=$env:GITHUB_SHA; target='x86_64-pc-windows-msvc'; agility=$version; files=[ordered]@{} }
    foreach ($item in Get-ChildItem -LiteralPath $stage -Recurse -File | Sort-Object FullName) {
        $key = $item.FullName.Substring($stage.Length+1).Replace('\','/')
        $manifest.files[$key] = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $stage 'BUILD.json') -Encoding UTF8
    $zip = Join-Path $dir 'VIctory-Equinox-Windows-x64.zip'
    $candidate = Join-Path $scratch 'tester.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($stage,$candidate)
    Move-Item -LiteralPath $candidate -Destination $zip -Force
    Write-Host "Windows x64 package: $zip"
} finally {
    if ([IO.Path]::GetDirectoryName($scratch).TrimEnd('\','/') -eq $temp.TrimEnd('\','/') -and [IO.Path]::GetFileName($scratch) -eq $name) {
        Remove-Item -LiteralPath $scratch -Recurse -Force
    }
}
