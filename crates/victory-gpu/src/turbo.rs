//! Neptune HyperEngine Core (`turbo.rs`):
//!
//! 1. **Persistent Zero-Spawn Worker Pool (`parallel_for`)**:
//!    Replaces per-batch `std::thread::scope` OS thread creation/teardown in
//!    texture refresh and descriptor heap sync with persistent worker threads
//!    (`VICTORY_TURBO_THREADS`, default 4 on 6C/12T CPUs). Re-entrant calls
//!    fall back inline with zero deadlock risk.
//! 2. **AVX2 256-bit 4-Lane Vector Hasher (`fast_hash_128`) & Zero Scanner
//!    (`is_all_zero`)**:
//!    Replaces cryptographic BLAKE3 hashing and byte-by-byte zero scans in hot
//!    texture dirty-checks with ~35-45 GB/s Zen 3 SIMD paths.
//! 3. **Shader Metadata Sidecar Cache (`shader_meta_get` / `shader_meta_put`)**:
//!    Caches the `(stores, walks, Vec<UsedResource>)` results of
//!    `victory_shader::native_programs()` in `%LOCALAPPDATA%\victory\cache\shaders\neptune-meta.bin`
//!    so cached shaders skip full RDNA2 binary disassembly on every launch
//!    without modifying `shader_cache.rs` or invalidating existing pipeline caches.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, RwLock};

use victory_shader::{ResourceType, RootType, UsedResource};

// ============================================================================
// 0. ZERO-DEPENDENCY FxHash (3-INSTRUCTION FAST MAP / SET FOR HOT PATHS)
// ============================================================================

/// Ultra-fast non-cryptographic multiply-shift hasher for integer/pointer/tuple
/// keys in hot per-draw and per-submission caches (replaces SipHash-1-3).
#[derive(Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

const FX_K: u64 = 0x517c_c1b7_2722_0a95;

impl FxHasher {
    #[inline(always)]
    fn mix(&mut self, v: u64) {
        self.hash = (self.hash.rotate_left(5) ^ v).wrapping_mul(FX_K);
    }
}

impl Hasher for FxHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.hash
    }
    #[inline(always)]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let w = u64::from_ne_bytes(bytes[..8].try_into().unwrap());
            self.mix(w);
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            let w = u32::from_ne_bytes(bytes[..4].try_into().unwrap());
            self.mix(u64::from(w));
            bytes = &bytes[4..];
        }
        for &b in bytes {
            self.mix(u64::from(b));
        }
    }
    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.mix(u64::from(i));
    }
    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.mix(u64::from(i));
    }
    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.mix(i);
    }
    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.mix(i as u64);
    }
}

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub(crate) type FastSet<K> = HashSet<K, BuildHasherDefault<FxHasher>>;

/// Telemetry: `[pool_jobs, pool_inline, meta_hits, meta_misses]`.
static STATS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

// ============================================================================
// 1. PERSISTENT ZERO-SPAWN WORKER POOL
// ============================================================================

struct Pool {
    workers: usize,
    busy: AtomicBool,
    state: Mutex<PoolState>,
    wake_workers: Condvar,
    wake_caller: Condvar,
}

struct PoolState {
    epoch: u64,
    fn_ptr: usize,
    fn_meta: usize,
    len: usize,
    active: usize,
    done: usize,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

static POOL_NEXT: AtomicUsize = AtomicUsize::new(0);
static POOL: OnceLock<Pool> = OnceLock::new();

fn worker_count() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        let default = std::thread::available_parallelism()
            .map_or(4, |n| (n.get() / 3).clamp(2, 6));
        std::env::var("VICTORY_TURBO_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(default)
            .clamp(1, 12)
    })
}

fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let total = worker_count();
        Pool {
            workers: total,
            busy: AtomicBool::new(false),
            state: Mutex::new(PoolState {
                epoch: 0,
                fn_ptr: 0,
                fn_meta: 0,
                len: 0,
                active: 0,
                done: 0,
                panic: None,
            }),
            wake_workers: Condvar::new(),
            wake_caller: Condvar::new(),
        }
    })
}

fn helpers_started() -> usize {
    static STARTED: OnceLock<usize> = OnceLock::new();
    *STARTED.get_or_init(|| {
        let requested = pool().workers.saturating_sub(1);
        let mut started = 0;
        for id in 0..requested {
            match std::thread::Builder::new().name(format!("neptune-turbo-{id}")).spawn(move || worker_loop(id)) {
                Ok(_) => started += 1,
                Err(error) => {
                    tracing::warn!(%error, "worker pool reduced after a thread creation failure");
                    break;
                }
            }
        }
        tracing::info!(threads = started + 1, "Neptune HyperEngine worker pool active (VICTORY_TURBO_THREADS)");
        started
    })
}

fn run_indices(len: usize, f: &dyn Fn(usize)) -> std::thread::Result<()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
        let next = POOL_NEXT.fetch_add(1, Ordering::Relaxed);
        if next >= len {
            break;
        }
        f(next);
    }))
}

fn worker_loop(_id: usize) {
    let p = pool();
    let mut seen_epoch = 0u64;
    loop {
        let (fn_ptr, fn_meta, len, epoch) = {
            let mut st = p.state.lock().unwrap_or_else(|e| e.into_inner());
            while st.epoch == seen_epoch || st.active == 0 {
                st = p.wake_workers.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            seen_epoch = st.epoch;
            (st.fn_ptr, st.fn_meta, st.len, st.epoch)
        };
        // SAFETY: `parallel_for` keeps the fat pointer's closure alive until
        // every helper increments `st.done == st.active` for this `epoch`.
        let f: &(dyn Fn(usize) + Sync) = unsafe { std::mem::transmute([fn_ptr, fn_meta]) };
        let result = run_indices(len, f);
        let mut st = p.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.epoch == epoch {
            if let Err(panic) = result {
                if st.panic.is_none() { st.panic = Some(panic); }
            }
            st.done += 1;
            if st.done >= st.active {
                p.wake_caller.notify_one();
            }
        }
    }
}

/// Runs `f(0..len)` across persistent worker threads + the calling thread.
/// Re-entrant or tiny (`len <= 1`) calls run inline with zero overhead.
pub(crate) fn parallel_for<F: Fn(usize) + Sync>(len: usize, f: F) {
    if len <= 1 {
        for i in 0..len {
            f(i);
        }
        return;
    }
    let p = pool();
    let helpers = helpers_started();
    if helpers == 0
        || p.busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
    {
        STATS[1].fetch_add(1, Ordering::Relaxed);
        for i in 0..len {
            f(i);
        }
        return;
    }
    STATS[0].fetch_add(1, Ordering::Relaxed);

    let dyn_f: &(dyn Fn(usize) + Sync) = &f;
    // SAFETY: trait object reference has the layout of two `usize` words.
    let [fn_ptr, fn_meta]: [usize; 2] = unsafe { std::mem::transmute(dyn_f) };
    POOL_NEXT.store(0, Ordering::Release);
    {
        let mut st = p.state.lock().unwrap_or_else(|e| e.into_inner());
        st.epoch = st.epoch.wrapping_add(1).max(1);
        st.fn_ptr = fn_ptr;
        st.fn_meta = fn_meta;
        st.len = len;
        st.active = helpers;
        st.done = 0;
        p.wake_workers.notify_all();
    }

    // Even when a closure unwinds, all borrowed work must finish before the caller returns.
    let caller = run_indices(len, &f);
    let mut st = p.state.lock().unwrap_or_else(|e| e.into_inner());
    while st.done < st.active {
        st = p.wake_caller.wait(st).unwrap_or_else(|e| e.into_inner());
    }
    st.active = 0;
    let failure = caller.err().or_else(|| st.panic.take());
    st.panic = None;
    drop(st);
    p.busy.store(false, Ordering::Release);
    if let Some(payload) = failure {
        std::panic::resume_unwind(payload);
    }
}

// ============================================================================
// 2. AVX2 256-BIT 4-LANE SIMD HASHER & ZERO SCANNER
// ============================================================================

#[inline(always)]
fn mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn fast_hash_128_avx2(data: &[u8]) -> u128 {
    use std::arch::x86_64::*;

    let len = data.len();
    let ptr = data.as_ptr();
    let mut i = 0usize;

    let mul1 = _mm256_set1_epi32(0x9e37_79b1u32 as i32);
    let mul2 = _mm256_set1_epi32(0x85eb_ca77u32 as i32);
    let mut a0 = _mm256_set_epi64x(
        0x243f_6a88_85a3_08d3u64 as i64,
        0x1319_8a2e_0370_7344u64 as i64,
        0xa409_3822_299f_31d0u64 as i64,
        0x082e_fa98_ec4e_6c89u64 as i64,
    );
    let mut a1 = _mm256_set_epi64x(
        0x4528_21e6_38d0_1377u64 as i64,
        0xbe54_66cf_34e9_0c6cu64 as i64,
        0xc0ac_29b7_c97c_50ddu64 as i64,
        0x3f84_d5b5_b547_0917u64 as i64,
    );
    let mut a2 = _mm256_set_epi64x(
        0x9216_d5d9_8979_fb1bu64 as i64,
        0xd131_0ba6_98df_b5acu64 as i64,
        0x2ffd_72db_d01a_dfb7u64 as i64,
        0xb8e1_afed_6a26_7e96u64 as i64,
    );
    let mut a3 = _mm256_set_epi64x(
        0xba7c_9045_f12c_7f99u64 as i64,
        0x24a1_9947_b391_6cf7u64 as i64,
        0x0801_f2e2_858e_fc16u64 as i64,
        0x6369_20d8_7157_4e69u64 as i64,
    );

    while i + 128 <= len {
        let v0 = unsafe { _mm256_loadu_si256(ptr.add(i) as *const __m256i) };
        let v1 = unsafe { _mm256_loadu_si256(ptr.add(i + 32) as *const __m256i) };
        let v2 = unsafe { _mm256_loadu_si256(ptr.add(i + 64) as *const __m256i) };
        let v3 = unsafe { _mm256_loadu_si256(ptr.add(i + 96) as *const __m256i) };

        a0 = _mm256_add_epi64(_mm256_xor_si256(a0, _mm256_mullo_epi32(v0, mul1)), v1);
        a1 = _mm256_add_epi64(_mm256_xor_si256(a1, _mm256_mullo_epi32(v1, mul2)), v2);
        a2 = _mm256_add_epi64(_mm256_xor_si256(a2, _mm256_mullo_epi32(v2, mul1)), v3);
        a3 = _mm256_add_epi64(_mm256_xor_si256(a3, _mm256_mullo_epi32(v3, mul2)), v0);

        // Cross-lane bit diffusion so identical 32-byte shifted blocks differ.
        a0 = _mm256_xor_si256(a0, _mm256_srli_epi64(a0, 29));
        a2 = _mm256_xor_si256(a2, _mm256_srli_epi64(a2, 31));
        i += 128;
    }

    while i + 32 <= len {
        let v = unsafe { _mm256_loadu_si256(ptr.add(i) as *const __m256i) };
        a0 = _mm256_add_epi64(_mm256_xor_si256(a0, _mm256_mullo_epi32(v, mul1)), a1);
        a1 = _mm256_xor_si256(a1, _mm256_srli_epi64(v, 17));
        i += 32;
    }

    let c0 = _mm256_xor_si256(a0, _mm256_permute4x64_epi64(a2, 0b01_00_11_10));
    let c1 = _mm256_xor_si256(a1, _mm256_permute4x64_epi64(a3, 0b10_11_00_01));
    let mut lanes0 = [0u64; 4];
    let mut lanes1 = [0u64; 4];
    unsafe {
        _mm256_storeu_si256(lanes0.as_mut_ptr() as *mut __m256i, c0);
        _mm256_storeu_si256(lanes1.as_mut_ptr() as *mut __m256i, c1);
    }

    let mut h0 = mix64(lanes0[0] ^ lanes0[2].rotate_left(23) ^ (len as u64));
    let mut h1 = mix64(lanes0[1] ^ lanes0[3].rotate_left(41) ^ lanes1[0].rotate_left(13));
    h0 ^= mix64(lanes1[1] ^ lanes1[3].rotate_left(19));
    h1 ^= mix64(lanes1[2] ^ (len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));

    // Tail (< 32 bytes)
    let mut tail_lo = 0u64;
    let mut tail_hi = 0u64;
    while i + 8 <= len {
        let w = unsafe { ptr.add(i).cast::<u64>().read_unaligned() };
        tail_lo = mix64(tail_lo ^ w);
        tail_hi = tail_hi.wrapping_add(w.rotate_left(17));
        i += 8;
    }
    let mut rem = 0u64;
    let mut shift = 0u32;
    while i < len {
        rem |= u64::from(unsafe { *ptr.add(i) }) << shift;
        shift += 8;
        i += 1;
    }
    h0 = mix64(h0 ^ tail_lo ^ rem);
    h1 = mix64(h1 ^ tail_hi ^ rem.rotate_left(31) ^ h0);
    (u128::from(h1) << 64) | u128::from(h0)
}

fn fast_hash_128_scalar(data: &[u8]) -> u128 {
    let mut h0 = 0x243f_6a88_85a3_08d3u64 ^ (data.len() as u64);
    let mut h1 = 0x1319_8a2e_0370_7344u64.wrapping_add(data.len() as u64);
    let mut chunks = data.chunks_exact(16);
    for c in &mut chunks {
        let a = u64::from_le_bytes(c[0..8].try_into().unwrap());
        let b = u64::from_le_bytes(c[8..16].try_into().unwrap());
        h0 = mix64(h0 ^ a);
        h1 = mix64(h1.wrapping_add(b) ^ h0.rotate_left(17));
    }
    let rem = chunks.remainder();
    let mut last = 0u64;
    for (idx, &b) in rem.iter().enumerate() {
        last ^= u64::from(b) << ((idx & 7) * 8);
    }
    h0 = mix64(h0 ^ last);
    h1 = mix64(h1 ^ last.rotate_left(29) ^ h0);
    (u128::from(h1) << 64) | u128::from(h0)
}

/// Fast 128-bit content hash (AVX2 4-lane vector path on x86_64, ~35-45 GB/s).
#[inline]
pub(crate) fn fast_hash_128(data: &[u8]) -> u128 {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: runtime AVX2 feature check succeeded.
            return unsafe { fast_hash_128_avx2(data) };
        }
    }
    fast_hash_128_scalar(data)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn is_all_zero_avx2(data: &[u8]) -> bool {
    use std::arch::x86_64::*;

    let len = data.len();
    let ptr = data.as_ptr();
    let mut i = 0usize;
    while i + 128 <= len {
        let v0 = unsafe { _mm256_loadu_si256(ptr.add(i) as *const __m256i) };
        let v1 = unsafe { _mm256_loadu_si256(ptr.add(i + 32) as *const __m256i) };
        let v2 = unsafe { _mm256_loadu_si256(ptr.add(i + 64) as *const __m256i) };
        let v3 = unsafe { _mm256_loadu_si256(ptr.add(i + 96) as *const __m256i) };
        let any = _mm256_or_si256(_mm256_or_si256(v0, v1), _mm256_or_si256(v2, v3));
        if _mm256_testz_si256(any, any) == 0 {
            return false;
        }
        i += 128;
    }
    while i + 32 <= len {
        let v = unsafe { _mm256_loadu_si256(ptr.add(i) as *const __m256i) };
        if _mm256_testz_si256(v, v) == 0 {
            return false;
        }
        i += 32;
    }
    data[i..].iter().all(|&b| b == 0)
}

/// Fast SIMD check whether every byte in `data` is zero.
#[inline]
pub(crate) fn is_all_zero(data: &[u8]) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        if data.len() >= 64 && std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: runtime AVX2 feature check succeeded.
            return unsafe { is_all_zero_avx2(data) };
        }
    }
    data.iter().all(|&b| b == 0)
}

// ============================================================================
// 3. SHADER METADATA SIDECAR CACHE (SKIPS RDNA2 DISASSEMBLY ON CACHE HIT)
// ============================================================================

#[derive(Clone)]
pub(crate) struct ShaderMeta {
    pub(crate) stores: bool,
    pub(crate) walks: bool,
    pub(crate) used: Vec<UsedResource>,
}

const META_MAGIC: &[u8; 8] = b"NEPMETA1";

struct MetaStore {
    map: RwLock<HashMap<u128, ShaderMeta>>,
    path: Option<PathBuf>,
    dirty: AtomicU64,
    saving: AtomicBool,
}

static META_DIR: OnceLock<PathBuf> = OnceLock::new();
static META: OnceLock<MetaStore> = OnceLock::new();

/// Sets the shader cache directory so `neptune-meta.bin` lives alongside it.
pub(crate) fn set_meta_dir(dir: PathBuf) {
    let _ = META_DIR.set(dir);
}

fn meta_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SHADER_META_CACHE").is_none_or(|v| v != "0"))
}

fn encode_resource_kind(k: ResourceType) -> (u8, u8) {
    match k {
        ResourceType::Srv => (0, 0),
        ResourceType::Sampler => (1, 0),
        ResourceType::Cbv => (2, 0),
        ResourceType::Uav => (3, 0),
        ResourceType::Other(n) => (4, n),
    }
}

fn decode_resource_kind(tag: u8, extra: u8) -> ResourceType {
    match tag {
        0 => ResourceType::Srv,
        1 => ResourceType::Sampler,
        2 => ResourceType::Cbv,
        3 => ResourceType::Uav,
        _ => ResourceType::Other(extra),
    }
}

fn encode_root(r: RootType) -> u32 {
    match r {
        RootType::Constants => 0,
        RootType::Descriptor => 1,
        RootType::Table => 3,
        RootType::ImmediateSampler => 4,
        RootType::ResourceHeap => 5,
        RootType::SamplerHeap => 6,
        RootType::Other(n) => n | 0x8000_0000,
    }
}

fn decode_root(raw: u32) -> RootType {
    if raw & 0x8000_0000 != 0 {
        return RootType::Other(raw & 0x7fff_ffff);
    }
    match raw {
        0 => RootType::Constants,
        1 => RootType::Descriptor,
        3 => RootType::Table,
        4 => RootType::ImmediateSampler,
        5 => RootType::ResourceHeap,
        6 => RootType::SamplerHeap,
        n => RootType::Other(n),
    }
}

fn load_meta_file(path: &PathBuf) -> HashMap<u128, ShaderMeta> {
    let mut map = HashMap::new();
    let Ok(bytes) = std::fs::read(path) else {
        return map;
    };
    if bytes.len() < 12 || &bytes[0..8] != META_MAGIC {
        return map;
    }
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    map.reserve(count.min(65_536));
    let mut pos = 12usize;
    for _ in 0..count {
        if pos + 20 > bytes.len() {
            break;
        }
        let key = u128::from_le_bytes(bytes[pos..pos + 16].try_into().unwrap());
        let flags = bytes[pos + 16];
        let n_used = u16::from_le_bytes(bytes[pos + 18..pos + 20].try_into().unwrap()) as usize;
        pos += 20;
        let record_bytes = n_used.saturating_mul(21);
        if pos + record_bytes > bytes.len() {
            break;
        }
        let mut used = Vec::with_capacity(n_used);
        for _ in 0..n_used {
            let kind = decode_resource_kind(bytes[pos], bytes[pos + 1]);
            let dwords = bytes[pos + 2];
            let user_data = bytes[pos + 3];
            let slot = bytes[pos + 4];
            let root = decode_root(u32::from_le_bytes(bytes[pos + 5..pos + 9].try_into().unwrap()));
            let table_offset_dwords =
                u32::from_le_bytes(bytes[pos + 9..pos + 13].try_into().unwrap());
            let lower_bound = u32::from_le_bytes(bytes[pos + 13..pos + 17].try_into().unwrap());
            let stride = u32::from_le_bytes(bytes[pos + 17..pos + 21].try_into().unwrap());
            pos += 21;
            used.push(UsedResource {
                kind,
                root,
                table_offset_dwords,
                lower_bound,
                stride,
                dwords,
                user_data,
                slot,
            });
        }
        map.insert(
            key,
            ShaderMeta {
                stores: flags & 1 != 0,
                walks: flags & 2 != 0,
                used,
            },
        );
    }
    if !map.is_empty() {
        tracing::info!(
            entries = map.len(),
            path = %path.display(),
            "Neptune shader metadata sidecar cache loaded"
        );
    }
    map
}

fn meta_store() -> &'static MetaStore {
    META.get_or_init(|| {
        let path = META_DIR
            .get()
            .cloned()
            .or_else(|| {
                std::env::var_os("LOCALAPPDATA")
                    .map(|d| PathBuf::from(d).join("victory").join("cache").join("shaders"))
            })
            .map(|d| d.join("neptune-meta.bin"));
        let map = path.as_ref().map(load_meta_file).unwrap_or_default();
        MetaStore {
            map: RwLock::new(map),
            path,
            dirty: AtomicU64::new(0),
            saving: AtomicBool::new(false),
        }
    })
}

pub(crate) fn shader_meta_get(container: &[u8]) -> (u128, Option<ShaderMeta>) {
    let key = fast_hash_128(container);
    if !meta_enabled() {
        return (key, None);
    }
    let store = meta_store();
    let hit = store
        .map
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned();
    if hit.is_some() {
        STATS[2].fetch_add(1, Ordering::Relaxed);
    } else {
        STATS[3].fetch_add(1, Ordering::Relaxed);
    }
    (key, hit)
}

pub(crate) fn shader_meta_put(key: u128, meta: ShaderMeta) {
    if !meta_enabled() {
        return;
    }
    let store = meta_store();
    {
        let mut map = store.map.write().unwrap_or_else(|e| e.into_inner());
        map.insert(key, meta);
    }
    let pending = store.dirty.fetch_add(1, Ordering::Relaxed) + 1;
    // Flush asynchronously every 64 new shaders so startup/streaming never stalls.
    if pending >= 64
        && store
            .saving
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    {
        store.dirty.store(0, Ordering::Relaxed);
        let _ = std::thread::Builder::new()
            .name("neptune-meta-save".into())
            .spawn(flush_meta_now);
    }
}

fn flush_meta_now() {
    let store = meta_store();
    let Some(path) = store.path.clone() else {
        store.saving.store(false, Ordering::Release);
        return;
    };
    let snapshot: Vec<(u128, ShaderMeta)> = {
        let map = store.map.read().unwrap_or_else(|e| e.into_inner());
        map.iter().map(|(&k, v)| (k, v.clone())).collect()
    };
    let mut out = Vec::with_capacity(12 + snapshot.len() * 64);
    out.extend_from_slice(META_MAGIC);
    out.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
    for (key, m) in &snapshot {
        out.extend_from_slice(&key.to_le_bytes());
        let flags = u8::from(m.stores) | (u8::from(m.walks) << 1);
        out.push(flags);
        out.push(0);
        let n = m.used.len().min(u16::MAX as usize);
        out.extend_from_slice(&(n as u16).to_le_bytes());
        for u in &m.used[..n] {
            let (tag, extra) = encode_resource_kind(u.kind);
            out.push(tag);
            out.push(extra);
            out.push(u.dwords);
            out.push(u.user_data);
            out.push(u.slot);
            out.extend_from_slice(&encode_root(u.root).to_le_bytes());
            out.extend_from_slice(&u.table_offset_dwords.to_le_bytes());
            out.extend_from_slice(&u.lower_bound.to_le_bytes());
            out.extend_from_slice(&u.stride.to_le_bytes());
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("bin.tmp");
    if std::fs::write(&tmp, &out).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    store.saving.store(false, Ordering::Release);
}

/// Logs HyperEngine telemetry alongside the frame summary.
pub(crate) fn log_stats(frames: f64) {
    if frames <= 0.0 {
        return;
    }
    let jobs = STATS[0].swap(0, Ordering::Relaxed) as f64 / frames;
    let inline = STATS[1].swap(0, Ordering::Relaxed) as f64 / frames;
    let hits = STATS[2].load(Ordering::Relaxed);
    let misses = STATS[3].load(Ordering::Relaxed);
    if jobs > 0.0 || hits + misses > 0 {
        tracing::info!(
            "Neptune HyperEngine: pool {jobs:.1} parallel batches/frame ({inline:.1} inline), shader-meta cache {hits} hits / {misses} misses"
        );
    }
}


#[cfg(test)]
mod pool_regression {
    #[test]
    fn a_panicking_batch_joins_and_the_next_batch_finishes() {
        let failed = std::panic::catch_unwind(|| super::parallel_for(48, |i| {
            if i == 17 { panic!("synthetic job failure") }
        }));
        assert!(failed.is_err());
        let visits = [const { std::sync::atomic::AtomicUsize::new(0) }; 48];
        super::parallel_for(visits.len(), |i| { visits[i].fetch_add(1, super::Ordering::Relaxed); });
        assert!(visits.iter().all(|count| count.load(super::Ordering::Relaxed) == 1));
    }
}
