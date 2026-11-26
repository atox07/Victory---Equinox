//! "Already checked in this submission" (`VICTORY_REFRESH_STAMP=1`; off by
//! default; from clankenjoyer/victory-fps 5e30085 & proneonn/Vertex 9c591832).
//!
//! Model: Ryujinx `MultiRegionHandle.QueryModified` (`_sequenceNumberBitmap`)
//! and `Buffer.SynchronizeMemory` (MIT).
//!
//! A world submission asks for ~20,000 refreshes of host copies, most of them
//! of ranges an earlier refresh of the same submission already brought up to
//! date. Each copy remembers by 64 KiB page what it checked under the current
//! sequence number ([`now`]), and a request wholly inside checked pages returns
//! immediately (`Batch::add`, `Shadow::update`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::watch::PAGE;

/// Whether the stamp is on (`VICTORY_REFRESH_STAMP=1`).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_REFRESH_STAMP").is_some_and(|v| v == "1");
        if on {
            tracing::info!(
                "refreshes of ranges a copy already checked in this submission return at once (VICTORY_REFRESH_STAMP=1)"
            );
        }
        on
    })
}

/// The sequence number; 0 is never current.
static SEQ: AtomicU64 = AtomicU64::new(1);

/// Title memory may have changed under every copy: what was checked must be
/// checked again.
pub(crate) fn advance() {
    SEQ.fetch_add(1, Ordering::AcqRel);
}

/// The current sequence number.
pub(crate) fn now() -> u64 {
    SEQ.load(Ordering::Acquire)
}

/// Per copy: which of its pages were checked under which sequence.
#[derive(Default)]
pub(crate) struct Stamp {
    /// The sequence `bits` belong to (0: none), read without the lock.
    seq: AtomicU64,
    bits: Mutex<Bits>,
}

#[derive(Default)]
struct Bits {
    seq: u64,
    /// One bit per page of the copy, from the copy's first page.
    words: Vec<u64>,
    /// Pages of the copy not checked under `seq`.
    unchecked: usize,
}

fn pages(start: usize, end: usize) -> (usize, usize) {
    (start / PAGE, end.div_ceil(PAGE) - start / PAGE)
}

impl Stamp {
    /// Whether `[from, to)` (inside `[start, end)`) was checked whole under `seq`.
    pub(crate) fn current(&self, seq: u64, from: usize, to: usize, start: usize, _end: usize) -> bool {
        if from >= to || self.seq.load(Ordering::Acquire) != seq {
            return false;
        }
        let b = self.bits.lock().unwrap_or_else(|e| e.into_inner());
        if b.seq != seq {
            return false;
        }
        if b.unchecked == 0 {
            return true;
        }
        let base = start / PAGE;
        all_set(&b.words, from / PAGE - base, (to - 1) / PAGE - base)
    }

    /// `[from, to)` (inside `[start, end)`) was checked under `seq`.
    pub(crate) fn mark(&self, seq: u64, from: usize, to: usize, start: usize, end: usize) {
        self.note(seq, now(), from, to, start, end);
    }

    fn note(&self, seq: u64, latest: u64, from: usize, to: usize, start: usize, end: usize) {
        if from >= to || latest != seq {
            return;
        }
        let (base, count) = pages(start, end);
        let mut p0 = from / PAGE;
        if (p0 * PAGE).max(start) < from {
            p0 += 1;
        }
        let mut p1 = to.div_ceil(PAGE);
        if p1 > p0 && ((p1 * PAGE).min(end)) > to {
            p1 -= 1;
        }
        if p0 >= p1 {
            return;
        }
        let mut b = self.bits.lock().unwrap_or_else(|e| e.into_inner());
        if b.seq > seq {
            return;
        }
        if b.seq != seq {
            b.seq = seq;
            b.words.clear();
            b.words.resize(count.div_ceil(64), 0);
            b.unchecked = count;
        }
        let new = set_range(&mut b.words, p0 - base, p1 - 1 - base);
        b.unchecked -= new.min(b.unchecked);
        self.seq.store(seq, Ordering::Release);
    }
}

fn all_set(words: &[u64], first: usize, last: usize) -> bool {
    let mut at = first;
    while at <= last {
        let (w, b) = (at / 64, at % 64);
        let n = (last - at + 1).min(64 - b);
        let mask = if n == 64 { u64::MAX } else { ((1u64 << n) - 1) << b };
        match words.get(w) {
            Some(&v) if v & mask == mask => {}
            _ => return false,
        }
        at += n;
    }
    true
}

fn set_range(words: &mut [u64], first: usize, last: usize) -> usize {
    let mut at = first;
    let mut new = 0;
    while at <= last {
        let (w, b) = (at / 64, at % 64);
        let n = (last - at + 1).min(64 - b);
        let mask = if n == 64 { u64::MAX } else { ((1u64 << n) - 1) << b };
        let Some(v) = words.get_mut(w) else {
            break;
        };
        new += (mask & !*v).count_ones() as usize;
        *v |= mask;
        at += n;
    }
    new
}
