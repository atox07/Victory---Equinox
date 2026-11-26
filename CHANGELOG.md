# Changelog

## 0.1.0-equinox.1 — 2026-11-26 — first source candidate

### Continued from the unfinished Mid-Range snapshot

- Inherited the integrated renderer, persistent CPU workers, descriptor updates, memory pools and streaming work.
- Retained adaptive pool limits, strict texture views, normalized rectangle attributes, legacy FP32 subnormal handling and fence lifetime management.
- Retained RAM-tier profiles, conditional in-place memory, full readback in MID-RANGE and optional gap coalescing.
- Retained local diagnostics, successful-Present intervals and synthetic statistics/privacy tests.

### Finishing pass

- Renamed the public project, artifact, launcher banner, version metadata and local session directory to VIctory Equinox. Internal Rust crate and executable names remain unchanged.
- Replaced the inherited upstream landing page with an accurate tester-oriented README. Added credits, integration notes and a reproducible benchmark protocol.
- Completed the existing packager's missing documentation inputs; retained the pinned SDK and its digest verification.
- Removed the stale BCPack binary exception from `.gitignore`. No table is shipped.
- Added dual PowerShell synthetic checks, a native synthetic launcher probe using paths with spaces, package hash verification and a corresponding-source artifact to the manual workflow.
- Prevented an incomplete prewarm stamp from permanently suppressing future Auto warm-up attempts.

### Validation status

Source archive integrity, JSON/TOML/YAML structure, profile-to-runtime setting names, package file inputs and text-only source checks were reviewed locally. Windows builds, PowerShell execution, D3D12 validation and gameplay performance are pending. No performance improvement is claimed for this candidate.
