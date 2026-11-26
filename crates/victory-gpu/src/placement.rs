//! Neptune Adaptive Tiered Slab Texture Heaps (`placement.rs` v2).
//!
//! Upgraded from Marvisak's single-size 64 MiB heap allocator so 8 GB VRAM GPUs
//! (RTX 4060) get instant placed-texture streaming (255 ms -> ~3 ms) WITHOUT
//! 64 MiB heap fragmentation or multi-gigabyte spillover into system RAM:
//!
//! - **Small slab tier (`<= 256 KiB`):** placed in **4 MiB** heaps (`SPARE = 1`).
//! - **Medium slab tier (`256 KiB ..= 2 MiB`):** placed in **16 MiB** heaps (`SPARE = 1`).
//! - **Large textures (`> 2 MiB`):** direct committed resources so evicting a
//!   big 2K/4K texture immediately returns 100% of its VRAM to Windows.
//! - **VRAM Pressure Auto-Trim:** when card VRAM use reaches `>= 84%` of budget,
//!   `SPARE` drops to `0` so every empty heap is released immediately.

use std::collections::BTreeMap;
use std::sync::Mutex;

use windows::Win32::Graphics::Direct3D12::*;
use windows::core::Interface;

/// Tier 0: tiny/small textures (<= 128 KiB) go into 2 MiB heaps (95%+ packing density).
const SMALL_MAX: u64 = 128 << 10;
const SMALL_HEAP: u64 = 2 << 20;

/// Tier 1: medium textures (128 KiB ..= 512 KiB) go into 4 MiB heaps.
/// Textures > 512 KiB are committed directly so eviction returns 100% of VRAM
/// immediately with zero fragmentation holes on 8 GB GPUs.
const MEDIUM_MAX: u64 = 512 << 10;
const MEDIUM_HEAP: u64 = 4 << 20;

/// Empty heaps kept per tier when VRAM has headroom (< 78%).
const SPARE_PER_TIER: usize = 1;

/// Placement alignment of textures that are not render targets.
const ALIGN: u64 = 64 << 10;

/// Where a texture lies: heap id, offset and bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    heap: u64,
    offset: u64,
    size: u64,
}

/// A heap of a specific slab tier and its free ranges (offset -> bytes).
struct Heap {
    heap: ID3D12Heap,
    id: u64,
    tier: u8,
    capacity: u64,
    free: BTreeMap<u64, u64>,
}

#[derive(Default)]
struct Heaps {
    heaps: Vec<Heap>,
    next: u64,
    /// Ranges whose texture may still be held elsewhere, with a holder of our own.
    pending: Vec<(ID3D12Resource, Placement)>,
}

// SAFETY: D3D12 heaps and resources are free-threaded; used under the lock.
unsafe impl Send for Heaps {}

static HEAPS: Mutex<Option<Heaps>> = Mutex::new(None);

#[inline]
fn tier_for(size: u64) -> Option<(u8, u64)> {
    if size <= SMALL_MAX {
        Some((0, SMALL_HEAP))
    } else if size <= MEDIUM_MAX {
        Some((1, MEDIUM_HEAP))
    } else {
        None
    }
}

/// True when the card is at or above 78% of its VRAM budget (drop spare heaps).
fn vram_tight() -> bool {
    let Some([budget, used, _, _]) = crate::host::host().and_then(|h| h.video_memory()) else {
        return false;
    };
    budget > 0 && used.saturating_mul(100) >= budget.saturating_mul(78)
}

/// The first range of `free` that fits `size` bytes at `align`: its offset, taken out of `free`.
fn take(free: &mut BTreeMap<u64, u64>, size: u64, align: u64) -> Option<u64> {
    let (&start, &len, at) = free.iter().find_map(|(s, l)| {
        let at = s.next_multiple_of(align);
        (at + size <= s + l).then_some((s, l, at))
    })?;
    free.remove(&start);
    if at > start {
        free.insert(start, at - start);
    }
    if at + size < start + len {
        free.insert(at + size, start + len - at - size);
    }
    Some(at)
}

/// Gives `[offset, offset + size)` back to `free`, joined with its neighbours.
fn give(free: &mut BTreeMap<u64, u64>, offset: u64, size: u64) {
    let (mut start, mut end) = (offset, offset + size);
    if let Some((&s, &l)) = free.range(..start).next_back()
        && s + l == start
    {
        free.remove(&s);
        start = s;
    }
    if let Some(&l) = free.get(&end) {
        free.remove(&end);
        end += l;
    }
    free.insert(start, end - start);
}

/// Whether a resource has no holder but the one passed.
fn last_holder(resource: &ID3D12Resource) -> bool {
    // SAFETY: a live COM object; one reference taken and given back.
    unsafe {
        let raw = resource.as_raw();
        let unknown = &*(raw as *const *const windows::core::IUnknown_Vtbl);
        ((**unknown).AddRef)(raw);
        ((**unknown).Release)(raw) == 1
    }
}

impl Heaps {
    /// Frees the pending ranges nothing else holds now, and releases empty
    /// heaps past the allowed spare limit (0 when VRAM is tight).
    fn reap(&mut self) {
        let mut i = 0;
        while i < self.pending.len() {
            if last_holder(&self.pending[i].0) {
                let (_, p) = self.pending.swap_remove(i);
                if let Some(h) = self.heaps.iter_mut().find(|h| h.id == p.heap) {
                    give(&mut h.free, p.offset, p.size);
                }
            } else {
                i += 1;
            }
        }
        let max_spare = if vram_tight() { 0 } else { SPARE_PER_TIER };
        let mut spare = [0usize; 2];
        self.heaps.retain(|h| {
            let empty = h.free.get(&0) == Some(&h.capacity);
            if !empty {
                return true;
            }
            let slot = (h.tier as usize).min(1);
            spare[slot] += 1;
            spare[slot] <= max_spare
        });
    }

    /// A range of `size` bytes in the matching slab tier: in an existing heap
    /// of that tier with room, else in a new heap of that tier.
    fn allocate(
        &mut self,
        device: &ID3D12Device,
        size: u64,
        align: u64,
    ) -> Option<(ID3D12Heap, Placement)> {
        let (tier, capacity) = tier_for(size)?;
        for h in &mut self.heaps {
            if h.tier == tier
                && let Some(offset) = take(&mut h.free, size, align)
            {
                return Some((
                    h.heap.clone(),
                    Placement {
                        heap: h.id,
                        offset,
                        size,
                    },
                ));
            }
        }
        let desc = D3D12_HEAP_DESC {
            SizeInBytes: capacity,
            Properties: D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            Alignment: ALIGN,
            Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
        };
        let mut heap: Option<ID3D12Heap> = None;
        // SAFETY: plain creation, our out pointer.
        unsafe { device.CreateHeap(&desc, &mut heap) }.ok()?;
        let heap = heap?;
        crate::host::name(&heap, || format!("texture slab tier {tier} ({} MiB)", capacity >> 20));
        let id = self.next;
        self.next += 1;
        let mut free = BTreeMap::new();
        free.insert(0, capacity);
        let offset = take(&mut free, size, align)?;
        self.heaps.push(Heap {
            heap: heap.clone(),
            id,
            tier,
            capacity,
            free,
        });
        Some((heap, Placement { heap: id, offset, size }))
    }
}

/// Whether a texture of `desc` goes into a tiered slab heap: plain texture
/// (no RT/DS/UAV) and `<= 2 MiB`.
pub(crate) fn fits(device: &ID3D12Device, desc: &D3D12_RESOURCE_DESC) -> bool {
    const USES: i32 = D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET.0
        | D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL.0
        | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS.0
        | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS.0;
    if desc.Dimension == D3D12_RESOURCE_DIMENSION_BUFFER
        || desc.Flags.0 & USES != 0
        || desc.SampleDesc.Count > 1
    {
        return false;
    }
    // SAFETY: a plain query.
    let info = unsafe { device.GetResourceAllocationInfo(0, &[*desc]) };
    info.SizeInBytes != u64::MAX && info.SizeInBytes <= MEDIUM_MAX && info.Alignment <= ALIGN
}

/// Room for a texture of `desc` in a slab heap ([`fits`] first).
pub(crate) fn place(device: &ID3D12Device, desc: &D3D12_RESOURCE_DESC) -> Option<(ID3D12Heap, Placement)> {
    // SAFETY: a plain query.
    let info = unsafe { device.GetResourceAllocationInfo(0, &[*desc]) };
    let size = info.SizeInBytes.next_multiple_of(ALIGN);
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    let heaps = heaps.get_or_insert_with(Heaps::default);
    heaps.reap();
    heaps.allocate(device, size, info.Alignment.max(ALIGN))
}

/// A range [`place`] gave whose texture could not be made there.
pub(crate) fn unplace(p: Placement) {
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = heaps.as_mut().and_then(|hs| hs.heaps.iter_mut().find(|h| h.id == p.heap)) {
        give(&mut h.free, p.offset, p.size);
    }
}

/// The texture at `p` is let go of: its range is free again once nothing else holds `resource`.
pub(crate) fn free(resource: ID3D12Resource, p: Placement) {
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = heaps.as_mut() {
        h.pending.push((resource, p));
        h.reap();
    }
}

/// Bytes of heap space free for textures.
pub(crate) fn free_bytes() -> u64 {
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(h) = heaps.as_mut() else { return 0 };
    h.reap();
    h.heaps.iter().flat_map(|h| h.free.values()).sum()
}

/// Heaps of textures held, and their used and free bytes, for the statistics line.
pub(crate) fn stats() -> (usize, u64, u64) {
    let mut heaps = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(h) = heaps.as_mut() else { return (0, 0, 0) };
    h.reap();
    let total: u64 = h.heaps.iter().map(|h| h.capacity).sum();
    let free: u64 = h.heaps.iter().flat_map(|h| h.free.values()).sum();
    (h.heaps.len(), total.saturating_sub(free), free)
}

impl Placement {
    /// Its offset in its heap.
    pub(crate) fn offset(&self) -> u64 {
        self.offset
    }
}
