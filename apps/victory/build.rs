//! Exports the two symbols by which `d3d12.dll` finds the redistributable
//! Agility SDK runtime (`D3D12\D3D12Core.dll` 1.620.2) beside `victory.exe`.
//! Needed on Windows 10 (whose inbox D3D12 stops at SM 6.5, rejecting all
//! SM 6.6 root signatures) in release builds as well as diagnostics builds.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:D3D12SDKVersion,DATA");
        println!("cargo:rustc-link-arg-bins=/EXPORT:D3D12SDKPath,DATA");
    }
}
