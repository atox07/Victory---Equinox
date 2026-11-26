# Credits and provenance

VIctory Equinox is a continuation of the provided unfinished Mid-Range source snapshot. Original code and notices remain under the inherited GPL-3.0-or-later license. The finishing pass changes distribution tooling and documentation, not the upstream authorship of the runtime.

## Direct integration references

- **Neptune HyperEngine / Dolberth** — persistent workers, memory and streaming architecture, built-in upscaling. Original attribution and historical performance claims: [HYPERENGINE.md](HYPERENGINE.md).
- **Vice-Tay / TaynanGT** — selective renderer/shader and fence corrections. Upstream repository: https://github.com/TaynanGT/Vice-Tay.
- **Neptune FPS / Neptune contributors** — performance-oriented runtime reference, including copy-back alternatives, parallel translation, and indirect validation. Exact source commit of the unfinished snapshot was not provided.

## Inherited upstream contributors

- **Marvisak / VIctory** and contributors — loader, runtime, D3D12 back end and shader translation: https://github.com/Marvisak/VIctory.
- **fxceless, myokex, soldsawn**, and contributors — core runtime and shader work.
- **Traffic, imgone, zm0kie** — Neptune family.
- **ad3lre**, **proneonn**, **tippythetaidum**, **avatarmanz**, **Belobug**, **ShakiraRED-dt** — inherited optimizations and rendering work.
- **Jonirulah, Flyander, AscilCH, ZSlayerHQ, JoshuuDev, skornuta, Trexies, clankenjoyer, rorodev6, PalmVI, DexNom, Joshua Odom, Spexivus, joroyt, tightfall**, and other contributors named in the retained upstream documentation.
- **ivall / XCL** — behavioral reference, as described by upstream; no new import of GPL-2.0-only code is made in this finishing pass.

This is not a complete commit-level attribution audit. Source archives do not carry full Git history; preserve upstream notices and recover exact revision provenance where available before broad redistribution. No endorsement or affiliation is implied.

The Direct3D Agility SDK redistributable in the generated binary package is a separate Microsoft component; its package notices are included by the packager. The source archive does not contain it. The BCPack table data is excluded. Consult each component's terms.
