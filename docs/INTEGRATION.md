# Integration and release review

## Scope

The input was `Unfinished.zip`, a 218-file source snapshot. The finishing pass intentionally preserves the Rust runtime and its existing profiles rather than doing another unvalidated merge. The public name is now Equinox. Runtime changes below describe code already present in the supplied snapshot, not newly measured results.

| Area | Observed in supplied tree | Decision |
| --- | --- | --- |
| CPU workers and caches | `turbo.rs`, `descriptor.rs`, shader metadata cache | Preserve; verify worker counts and stalls on target |
| Texture descriptors | `descriptor/host.rs`, strict texture view compatibility | Preserve profile toggle |
| Rectangle attributes | `primitive.rs`, `VICTORY_RECT_NORMALIZED` | Preserve MID-RANGE setting |
| Floating-point behavior | `emit/lower.rs`, `VICTORY_LEGACY_MUL_FTZ` and NaN guard | Preserve; compare image fidelity |
| Fence lifetime | `fence.rs`, `FenceRef`, queue ownership | Preserve; verify device errors |
| VRAM pools | `shadow.rs`, adaptive pool budgets, RAM tier JSON | Preserve; measure actual DXGI and process memory |
| Transfer coalescing | `coalesce.rs`, blocked-range checks | Leave opt-in at 0 KiB by default |
| Copy-back | `smart_back.rs`, full readback profile | Keep Smart filtering off in MID-RANGE; direct CPU access coverage remains unresolved |
| Indirect commands | `draw.rs`, checked and CPU-validated modes | Keep MID-RANGE checked mode 0 |
| In-place memory | `inplace.rs`, `tools/Settings.ps1` | Preserve adaptive PCIe gate; unknown link defaults off |
| BCPack | `bcpack.rs`, external configured tables and synthetic fixtures | No proprietary table included |

No claim is made that the full Neptune FPS lazy fault-driven copy-back was integrated, or that every Vice-Tay rendering change was ported. The shipped source contains selected integration work. Full fault coverage and equivalent GPU/CPU readback remain an open engineering task. Avoid adopting aggressive switches solely to raise a counter.

## Release finishing work

The input already included a manual Windows workflow, a package script and PowerShell tests, but the packager referenced four absent documentation files. These inputs now exist. The landing page describes the new launcher rather than upstream defaults. CI also checks both PowerShell editions, tests a synthetic native program with spaces in paths, verifies the contents of the generated binary ZIP and provides a separate source artifact. Original profile values and Rust sources were not changed by the finishing pass.

## Verification boundaries

The previous session reported PowerShell synthetic test success. That report is not independently reproduced here. The local environment is Linux without Rust/MSVC or PowerShell executables; only static checks were possible. The manual CI must establish whether the Rust tree compiles. A passing CI cannot establish visual fidelity or gameplay correctness. Use the benchmark protocol and compare readback-dependent behavior, crash logs, long-session memory growth and actual performance. Do not treat any upstream FPS figures as Equinox measurements.
