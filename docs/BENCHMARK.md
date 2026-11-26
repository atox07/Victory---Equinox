# Equinox benchmark protocol

## Goals

First establish that the release is functional. Target milestones are 15, 20 and 30 real FPS on the primary 8 GB GPU / 16 GB RAM configuration. These are goals, not established results. Test only authorized local inputs; do not distribute input data or raw logs.

## Record the environment

Record CPU, GPU/VRAM, RAM, PCIe generation and width under load, Windows version, driver, build SHA-256, console model, output/internal resolution, upscaler, RAM tier, cache state and profile. Keep page-file settings and storage unchanged across runs. Confirm the same runtime version and title revision, and check the actual worker counts.

## A/B procedure

1. Use one repeatable scenario with an identical starting state. Keep a private backup of the local data. Compare the same scene, path, camera, duration and settings. The default BASELINE window differs from MID-RANGE: explicitly set the same output size. Disable SmartCopyBack in BASELINE for a correctness-first comparison; record this override. A separate untouched original HyperEngine build is needed for an upstream comparison.
2. Run an initial cold-cache check separately, then prepare each profile's shader cache. Do not compare one cold run to one warm run. Record refused shader/pipeline counts and whether warm-up was complete.
3. Run at least three repeated warm samples per profile. Use 120 seconds of matched gameplay after a fixed settling interval. Avoid menu/loading/cutscene intervals unless separately measured. Record the starting timestamp relative to first successful Present.
4. Test several workloads: stationary view, motion through a dense area, rapidly changing scenery, and a scene with CPU/GPU readback-dependent physics. Compare visual correctness, collision/vehicle behavior, water/boat response and progression independently of FPS.
5. Perform a 30–60 minute long-session check. Observe VRAM budget/use, shared GPU memory, process commit, available RAM, paging, frame pacing, stutters, device removal and whether memory returns toward a plateau when revisiting areas. Do not infer a leak from a single high-water mark.
6. Export the numeric session diagnostics using `Diagnose.ps1` and `-StartSecond`/`-DurationSeconds` to isolate each measured window. System samples use process-relative time and UTC, whereas the frame samples use first-Present-relative time: align using the available timing metadata or compare whole-session memory statistics; do not assume both zero points coincide.
7. First use a normal release without GPU timing/DRED overhead for FPS comparisons; reproduce failures with the instrumented build separately. Test optional gap coalescing/in-place overrides one at a time. Keep the original run as a reference.

## Report template

| Metric | BASELINE | MID-RANGE |
| --- | --- | --- |
| Build hash / scenario / settings | | |
| Valid Present intervals / measured seconds | | |
| Mean FPS / 1% low | | |
| P95 / P99 frametime (ms) | | |
| Peak dedicated / shared GPU memory and budget | | |
| Peak working set / private commit / system commit | | |
| Paging and upload / copy-back times | | |
| Visible anomalies / physics / water | | |
| Errors / crashes / session duration | | |

The diagnostic mean FPS is total intervals divided by summed interval duration. The 1% low is the reciprocal of the mean of the slowest ceil(1%) intervals. P95/P99 use nearest ranks. Successful Present counts can include repeated or incomplete visual frames; confirm actual motion and rendering independently. Numeric log counters are heuristics, not proof of absence of errors. Review the sanitized ZIP before sharing. Do not share private raw logs, dumps, assets or proprietary tables.
