//! Transposition table (Large variant).
//!
//! Ported from
//! [`TransTableL.cpp`](../../../ddss-sys/vendor/src/TransTableL.cpp).
//!
//! # Overview
//!
//! The transposition table memoizes positions reached during the
//! alpha-beta search. Each position is keyed by:
//!
//! 1. The current trick and leading hand → selects a `TTroot[t][h]`
//!    bucket array of 256 entries.
//! 2. The packed `hand_dist` (suit lengths per hand) → indexed by
//!    [`hash8`] into that bucket array, giving a `DistHash` with up to
//!    32 distinct distributions that hash to the same slot.
//! 3. The suit-distribution `key` (the 48-bit packing of `hand_dist`)
//!    → selects one [`WinBlock`] within that 32-entry list.
//! 4. The "compressed cards" matched in 4 levels of mask/set pairs
//!    inside the `WinBlock` (up to 125 entries) — this is where the
//!    `aggr_target` and `win_ranks` of the position are checked.
//!
//! # Storage substrate
//!
//! `WinBlock`s live in a `Vec<Page>` where each page is a boxed slab of
//! [`BLOCKS_PER_PAGE`] (= 1000) blocks. Pages are added on demand up to
//! `pages_maximum`; [`TransTable::reset`] drops pages above
//! `pages_default`. A simple bump-pointer (page index + offset) inside
//! the most recent page hands out new blocks.
//!
//! # Divergences from the vendor
//!
//! - The harvest/age-based eviction path is removed. When the table
//!   reaches `pages_maximum` and a new block is requested, we perform
//!   a full reset instead. The vendor itself falls back to a full reset
//!   when harvest fails; this simplification trades a small hit-rate
//!   loss in long searches for a much smaller, `unsafe`-free
//!   implementation.
//! - All debug/printing methods (`PrintSuits`, `PrintEntries`, …) are
//!   omitted. They're only used by the vendor's CLI dump tooling.
//! - Memory ownership is handled by `Vec<Page>` rather than `malloc` /
//!   `free`; the `poolType` doubly-linked list is replaced with a plain
//!   `Vec<Page>` (push-only, truncate on reset).

// ---------- Vendor compile-time constants ----------------------------

const BLOCKS_PER_PAGE: usize = 1000;
const DISTS_PER_ENTRY: usize = 32;
const BLOCKS_PER_ENTRY: usize = 125;

const TT_BYTES: usize = 4;
const TT_TRICKS: usize = 12;
const TT_HANDS: usize = 4;
const TT_SUITS: usize = 4;
const TT_HASH_BUCKETS: usize = 256;

/// Default per-instance memory budget in MiB (matches `THREADMEM_LARGE_DEF_MB`).
pub(crate) const DEFAULT_MEMORY_MB: u32 = 95;
/// Maximum per-instance memory budget in MiB (matches `THREADMEM_LARGE_MAX_MB`).
pub(crate) const MAX_MEMORY_MB: u32 = 160;

// ---------- Public types ---------------------------------------------

/// A cached evaluation: bounds on the max-player's tricks plus the
/// best-move hint that produced them.
///
/// Mirrors the vendor's `nodeCardsType`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NodeCards {
    /// Upper bound on tricks for MAX from this position.
    pub ubound: i8,
    /// Lower bound on tricks for MAX from this position.
    pub lbound: i8,
    /// Suit of the best move (0..=3) — 0 if no hint stored.
    pub best_move_suit: u8,
    /// Rank of the best move (2..=14) — 0 if no hint stored.
    pub best_move_rank: u8,
    /// `least_win[s]` — encoded as `15 - lowest_relative_rank` of the
    /// suit `s` that was needed to evaluate this position. Vendor calls
    /// this `leastWin`. Index 0 means "void suit needed".
    pub least_win: [u8; TT_SUITS],
}

// ---------- Internal types -------------------------------------------

/// One entry in a `WinBlock`'s match list. ~52 bytes.
#[derive(Clone, Copy, Debug, Default)]
struct WinMatch {
    /// XOR of the per-suit `aggrRanks` for the cards involved — used by
    /// `CreateOrUpdate` to test for exact-match overwrite.
    xor_set: u32,
    /// 4 levels of "set" bits — the actual hand-position pattern.
    top_set: [u32; TT_BYTES],
    /// 4 levels of "mask" bits — which cards in `top_set` are live.
    top_mask: [u32; TT_BYTES],
    /// Packed `(low[0]<<12)|(low[1]<<8)|(low[2]<<4)|low[3]` — disambiguates
    /// matches with the same `xor_set` but different lowest-relevant rank.
    mask_index: i32,
    /// 1..=4 — the highest level (1-indexed) at which `top_mask` is
    /// non-zero. Lets `LookupCards` short-circuit comparisons.
    last_mask_no: i32,
    /// The cached node bounds + move hint.
    first: NodeCards,
}

/// 125-entry list of matches sharing a `(trick, hand, hash, hand_dist)`
/// tuple. Sized at ~6.5 KiB per block (BLOCKS_PER_ENTRY * sizeof(WinMatch)).
#[derive(Clone, Copy, Debug)]
struct WinBlock {
    /// One past the last index used during matching (filled in slot order).
    next_match_no: i32,
    /// Next slot to write to. May wrap around when the block is full.
    next_write_no: i32,
    /// Vendor's timestamp on last read, used by Harvest. We keep it for
    /// porting fidelity but never evict.
    timestamp_read: i32,
    list: [WinMatch; BLOCKS_PER_ENTRY],
}

impl WinBlock {
    fn new() -> Self {
        Self {
            next_match_no: 0,
            next_write_no: 0,
            timestamp_read: 0,
            list: [WinMatch::default(); BLOCKS_PER_ENTRY],
        }
    }

    fn reset(&mut self) {
        self.next_match_no = 0;
        self.next_write_no = 0;
        self.timestamp_read = 0;
    }
}

/// One variant within a hash bucket: maps a `key` (packed `hand_dist`)
/// to a `WinBlock` slot.
#[derive(Clone, Copy, Debug, Default)]
struct PosSearch {
    /// Slot index in the page pool, or `INVALID_BLOCK` if unused.
    pos_block: BlockId,
    key: i64,
}

/// One 256-bucket array indexed by `hash8(hand_dist)`. ~520 bytes
/// each, so 32 distributions per bucket cover collisions.
#[derive(Clone, Copy, Debug)]
struct DistHash {
    /// One past the last valid entry in `list` (during read).
    next_no: i32,
    /// Next index to write to (may wrap).
    next_write_no: i32,
    list: [PosSearch; DISTS_PER_ENTRY],
}

impl Default for DistHash {
    fn default() -> Self {
        Self {
            next_no: 0,
            next_write_no: 0,
            list: [PosSearch::default(); DISTS_PER_ENTRY],
        }
    }
}

/// One bucket array of 256 entries — kept boxed so the per-(trick,hand)
/// roots don't blow the stack (`12 * 4 * 256 * 520 B` ≈ 6.4 MiB).
type DistHashBuckets = Box<[DistHash; TT_HASH_BUCKETS]>;

/// Precomputed per-instance aggregator data — depends on the actual
/// `handLookup` so it can't live in a global `LazyLock`.
#[derive(Clone, Copy, Debug, Default)]
struct Aggr {
    /// `aggrRanks[s]` — per-suit running XOR seed of card-by-hand bits.
    aggr_ranks: [u32; TT_SUITS],
    /// `aggrBytes[s][b]` — pre-shifted bytes of `aggr_ranks` for the
    /// four packing levels.
    aggr_bytes: [[u32; TT_BYTES]; TT_SUITS],
}

/// One page of `BLOCKS_PER_PAGE` blocks, heap-allocated as a flat slab.
type Page = Box<[WinBlock; BLOCKS_PER_PAGE]>;

/// Opaque block reference into the page pool. Wrapped in
/// [`Option<BlockId>`] at use sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BlockId(u32);

impl BlockId {
    fn from_indices(page: u32, slot: u32) -> Self {
        Self(page * BLOCKS_PER_PAGE as u32 + slot)
    }

    fn page(self) -> u32 {
        self.0 / BLOCKS_PER_PAGE as u32
    }

    fn slot(self) -> u32 {
        self.0 % BLOCKS_PER_PAGE as u32
    }
}

// ---------- Static constants computed once ---------------------------

/// `TT_LOWEST_RANK[aggr]` — relative rank of the lowest card present
/// in `aggr`, where 14 is the lowest singleton bit. Empty suit is 15.
/// Different from the absolute lowest rank in `lookup::LOWEST_RANK`.
static TT_LOWEST_RANK: std::sync::LazyLock<Box<[i32; 8192]>> = std::sync::LazyLock::new(|| {
    let mut t = Box::new([0i32; 8192]);
    t[0] = 15;
    let mut top_bit_rank: usize = 1;
    for ind in 1..8192 {
        if ind >= (top_bit_rank << 1) {
            top_bit_rank <<= 1;
        }
        t[ind] = t[ind ^ top_bit_rank] - 1;
    }
    t
});

/// `MASK_BYTES[aggr][suit][b]` — precomputed mask bits for a 13-bit
/// `aggr`, sliced into the same 4-level layout as `aggr_bytes`.
///
/// Each suit needs 8 bits in one of 4 byte positions of a 32-bit
/// integer; this table pre-shifts the "all live" mask for each cell.
static MASK_BYTES: std::sync::LazyLock<Box<[[[u32; TT_BYTES]; TT_SUITS]; 8192]>> =
    std::sync::LazyLock::new(|| {
        let mut t = Box::new([[[0u32; TT_BYTES]; TT_SUITS]; 8192]);
        let mut win_mask = vec![0u32; 8192];
        let mut top_bit_rank: usize = 1;

        for ind in 1..8192 {
            if ind >= (top_bit_rank << 1) {
                top_bit_rank <<= 1;
            }
            // winMask grows by 2 bits per set rank — always 2*k ones
            // followed by zeros.
            win_mask[ind] = (win_mask[ind ^ top_bit_rank] >> 2) | (3 << 24);

            let w = win_mask[ind];
            t[ind][0][0] = (w << 6) & 0xff00_0000;
            t[ind][0][1] = (w << 14) & 0xff00_0000;
            t[ind][0][2] = (w << 22) & 0xff00_0000;
            t[ind][0][3] = (w << 30) & 0xff00_0000;

            t[ind][1][0] = (w >> 2) & 0x00ff_0000;
            t[ind][1][1] = (w << 6) & 0x00ff_0000;
            t[ind][1][2] = (w << 14) & 0x00ff_0000;
            t[ind][1][3] = (w << 22) & 0x00ff_0000;

            t[ind][2][0] = (w >> 10) & 0x0000_ff00;
            t[ind][2][1] = (w >> 2) & 0x0000_ff00;
            t[ind][2][2] = (w << 6) & 0x0000_ff00;
            t[ind][2][3] = (w << 14) & 0x0000_ff00;

            t[ind][3][0] = (w >> 18) & 0x0000_00ff;
            t[ind][3][1] = (w >> 10) & 0x0000_00ff;
            t[ind][3][2] = (w >> 2) & 0x0000_00ff;
            t[ind][3][3] = (w << 6) & 0x0000_00ff;
        }
        t
    });

// ---------- The transposition table ---------------------------------

/// Per-thread Large transposition table.
///
/// One instance per search context (vendor's `TransTableL`). Not
/// `Send`/`Sync` — must be owned by a single search worker.
pub(crate) struct TransTable {
    // ---- Memory budget ----
    pages_default: u32,
    pages_maximum: u32,

    // ---- Storage substrate ----
    /// All allocated pages. Indices into this are page numbers; each
    /// page holds [`BLOCKS_PER_PAGE`] blocks.
    pages: Vec<Page>,
    /// Slot index within the most recent page to allocate next. When
    /// this hits [`BLOCKS_PER_PAGE`] we need a fresh page.
    next_slot: u32,

    // ---- Hash table root ----
    /// `tt_root[trick][hand]` — 256-entry bucket array, always allocated.
    tt_root: [[DistHashBuckets; TT_HANDS]; TT_TRICKS],
    /// Last block looked at — set by `lookup`, read by `add`. `None`
    /// after `reset()`.
    last_block_seen: [[Option<BlockId>; TT_HANDS]; TT_TRICKS],

    // ---- Per-instance precomputed aggregator ----
    /// One [`Aggr`] per 13-bit `aggr_target` value. Filled in by
    /// [`Self::init`]; until then, all entries are zero and the table
    /// only supports independent-of-handLookup tests.
    aggr: Box<[Aggr; 8192]>,
    /// Whether `init` has been called.
    aggr_ready: bool,

    /// Read counter used for `timestamp_read` updates — preserved from
    /// vendor for porting fidelity (and useful if we re-introduce
    /// eviction).
    timestamp: i32,
}

impl Default for TransTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TransTable {
    /// Construct a table with the default memory limits
    /// ([`DEFAULT_MEMORY_MB`] / [`MAX_MEMORY_MB`]).
    pub(crate) fn new() -> Self {
        Self::with_memory(DEFAULT_MEMORY_MB, MAX_MEMORY_MB)
    }

    /// Construct a table with explicit memory limits in MiB.
    pub(crate) fn with_memory(default_mb: u32, max_mb: u32) -> Self {
        Self {
            pages_default: Self::mb_to_pages(default_mb),
            pages_maximum: Self::mb_to_pages(max_mb),
            pages: Vec::new(),
            next_slot: BLOCKS_PER_PAGE as u32, // forces a fresh page on first alloc
            tt_root: std::array::from_fn(|_| {
                std::array::from_fn(|_| {
                    vec![DistHash::default(); TT_HASH_BUCKETS]
                        .into_boxed_slice()
                        .try_into()
                        .unwrap_or_else(|_| unreachable!())
                })
            }),
            last_block_seen: [[None; TT_HANDS]; TT_TRICKS],
            aggr: Box::new([Aggr::default(); 8192]),
            aggr_ready: false,
            timestamp: 0,
        }
    }

    /// Compute the page budget for a given MiB ceiling, matching
    /// `SetMemoryDefault` / `SetMemoryMaximum` in the vendor.
    fn mb_to_pages(megabytes: u32) -> u32 {
        const WIN_BLOCK_BYTES: usize = std::mem::size_of::<WinBlock>();
        // block_mem (KiB) = BLOCKS_PER_PAGE * sizeof(WinBlock) / 1024
        let block_kib = (BLOCKS_PER_PAGE * WIN_BLOCK_BYTES) as f64 / 1024.0;
        ((1024.0 * megabytes as f64) / block_kib) as u32
    }

    /// Update the default memory budget. Doesn't immediately shrink the
    /// pool — that happens at the next [`Self::reset`].
    pub(crate) fn set_memory_default(&mut self, megabytes: u32) {
        self.pages_default = Self::mb_to_pages(megabytes);
    }

    /// Update the maximum memory budget. Doesn't immediately shrink the
    /// pool. New allocations beyond the new ceiling will trigger a reset.
    pub(crate) fn set_memory_maximum(&mut self, megabytes: u32) {
        self.pages_maximum = Self::mb_to_pages(megabytes);
    }

    /// Initialize the per-instance `aggr` table from `hand_lookup`.
    ///
    /// `hand_lookup[suit][rank]` — the hand (0..=3) that holds the
    /// absolute rank `rank` (2..=14) in `suit`. Mirrors the vendor's
    /// `TransTableL::Init(handLookup)`.
    ///
    /// Must be called before [`Self::lookup`] / [`Self::add`] if the
    /// search is to find hits. Untrained tables still work — they just
    /// match identically-zero positions.
    pub(crate) fn init(&mut self, hand_lookup: &[[i32; 15]; TT_SUITS]) {
        // The 0 entry is all zeros (already the default).
        for s in 0..TT_SUITS {
            self.aggr[0].aggr_ranks[s] = 0;
            for b in 0..TT_BYTES {
                self.aggr[0].aggr_bytes[s][b] = 0;
            }
        }

        let mut top_bit_rank: usize = 1;
        let mut top_bit_no: usize = 2;

        for ind in 1..8192 {
            if ind >= (top_bit_rank << 1) {
                top_bit_rank <<= 1;
                top_bit_no += 1;
            }

            self.aggr[ind] = self.aggr[ind ^ top_bit_rank];

            for s in 0..TT_SUITS {
                let h = hand_lookup[s][top_bit_no] as u32;
                self.aggr[ind].aggr_ranks[s] = (self.aggr[ind].aggr_ranks[s] >> 2) | (h << 24);
            }

            let ar = self.aggr[ind].aggr_ranks;
            let ab = &mut self.aggr[ind].aggr_bytes;

            ab[0][0] = (ar[0] << 6) & 0xff00_0000;
            ab[0][1] = (ar[0] << 14) & 0xff00_0000;
            ab[0][2] = (ar[0] << 22) & 0xff00_0000;
            ab[0][3] = (ar[0] << 30) & 0xff00_0000;

            ab[1][0] = (ar[1] >> 2) & 0x00ff_0000;
            ab[1][1] = (ar[1] << 6) & 0x00ff_0000;
            ab[1][2] = (ar[1] << 14) & 0x00ff_0000;
            ab[1][3] = (ar[1] << 22) & 0x00ff_0000;

            ab[2][0] = (ar[2] >> 10) & 0x0000_ff00;
            ab[2][1] = (ar[2] >> 2) & 0x0000_ff00;
            ab[2][2] = (ar[2] << 6) & 0x0000_ff00;
            ab[2][3] = (ar[2] << 14) & 0x0000_ff00;

            ab[3][0] = (ar[3] >> 18) & 0x0000_00ff;
            ab[3][1] = (ar[3] >> 10) & 0x0000_00ff;
            ab[3][2] = (ar[3] >> 2) & 0x0000_00ff;
            ab[3][3] = (ar[3] << 6) & 0x0000_00ff;
        }
        self.aggr_ready = true;
    }

    /// Reset hash table to empty without freeing the boxes.
    fn init_tt(&mut self) {
        for t in 0..TT_TRICKS {
            for h in 0..TT_HANDS {
                let buckets = &mut self.tt_root[t][h];
                for i in 0..TT_HASH_BUCKETS {
                    buckets[i].next_no = 0;
                    buckets[i].next_write_no = 0;
                }
                self.last_block_seen[t][h] = None;
            }
        }
    }

    /// Reset to "between solves" state — drops pages above
    /// `pages_default`, clears all hash entries and `last_block_seen`,
    /// and resets the bump pointer to the start of page 0.
    pub(crate) fn reset(&mut self) {
        // Mirror ResetMemory: keep `pages_default` pages, truncate the
        // rest, reset bump pointer.
        if self.pages.is_empty() {
            self.init_tt();
            self.timestamp = 0;
            return;
        }

        if self.pages.len() as u32 > self.pages_default {
            self.pages.truncate(self.pages_default as usize);
        }
        // Reset bump pointer to start of page 0 — but only if we have at
        // least one page kept. Otherwise next allocation will create one.
        self.next_slot = if self.pages.is_empty() {
            BLOCKS_PER_PAGE as u32
        } else {
            // Pretend we're at the END of the highest kept page → next
            // alloc reuses page 0. We emulate this by setting next_slot
            // = 0 and truncating to length 1 (then push pages 1..n-1
            // when we exhaust). Easier: just drop all pages, since the
            // vendor's "keep pages_default" optimization is for cold
            // memory; correctness only requires that we have *room* for
            // pages_default.
            //
            // For porting fidelity: keep one page, reset bump to 0.
            self.pages.truncate(1);
            0
        };

        self.init_tt();
        self.timestamp = 0;
    }

    // ---- Block allocation ------------------------------------------

    /// Return a fresh block, growing the pool by a page if needed. May
    /// trigger a full `reset()` when the budget is exhausted (see
    /// "Divergences from the vendor" in the module docs).
    fn get_next_card_block(&mut self) -> BlockId {
        // Common path: bump the slot within the current last page.
        if self.next_slot < BLOCKS_PER_PAGE as u32 {
            let page_idx = (self.pages.len() - 1) as u32;
            let slot = self.next_slot;
            self.next_slot += 1;
            let block_id = BlockId::from_indices(page_idx, slot);
            // Initialize the block to default — the vendor uses
            // malloc'd (uninitialized) memory but always overwrites
            // before reading; we're explicit here to keep `unsafe` out.
            self.pages[page_idx as usize][slot as usize].reset();
            return block_id;
        }

        // Need a new page. Three cases:
        //   1. We're below the maximum → allocate one.
        //   2. At maximum → reset (drops back to pages_default) and
        //      try again.
        if (self.pages.len() as u32) < self.pages_maximum {
            self.pages.push(Self::new_page());
            self.next_slot = 1;
            let page_idx = (self.pages.len() - 1) as u32;
            let block_id = BlockId::from_indices(page_idx, 0);
            self.pages[page_idx as usize][0].reset();
            block_id
        } else {
            // Out of budget. Vendor would harvest; we just reset.
            self.reset();
            // After reset, pages.len() <= pages_default. Allocate
            // something fresh.
            if self.pages.is_empty() {
                self.pages.push(Self::new_page());
                self.next_slot = 0;
            }
            // Recurse — we now have room.
            self.get_next_card_block()
        }
    }

    fn new_page() -> Page {
        // Construct a slab via Vec → boxed slice → array conversion.
        let v = vec![WinBlock::new(); BLOCKS_PER_PAGE];
        v.into_boxed_slice()
            .try_into()
            .unwrap_or_else(|_| unreachable!())
    }

    // ---- Block access ----------------------------------------------

    #[inline]
    fn block(&self, id: BlockId) -> &WinBlock {
        &self.pages[id.page() as usize][id.slot() as usize]
    }

    #[inline]
    fn block_mut(&mut self, id: BlockId) -> &mut WinBlock {
        &mut self.pages[id.page() as usize][id.slot() as usize]
    }

    // ---- Hash ------------------------------------------------------

    /// Vendor's `hash8` — collapses 4×12-bit hand distributions into a
    /// single 8-bit hash. Kept byte-identical to preserve cache hit rate.
    #[inline]
    fn hash8(hand_dist: &[i32; TT_HANDS]) -> usize {
        let h = (hand_dist[0] as i64)
            ^ ((hand_dist[1] as i64).wrapping_mul(5))
            ^ ((hand_dist[2] as i64).wrapping_mul(25))
            ^ ((hand_dist[3] as i64).wrapping_mul(125));
        let h = h as i32;
        ((h ^ (h >> 5)) & 0xff) as usize
    }

    /// Pack 4 `hand_dist` entries into a 48-bit key (rest zero).
    #[inline]
    fn suit_lengths_key(hand_dist: &[i32; TT_HANDS]) -> i64 {
        ((hand_dist[0] as i64) << 36)
            | ((hand_dist[1] as i64) << 24)
            | ((hand_dist[2] as i64) << 12)
            | (hand_dist[3] as i64)
    }

    // ---- Lookup ----------------------------------------------------

    /// Look up the position `(trick, hand, aggr_target, hand_dist)` in
    /// the table. If found and the bounds prove the search result
    /// (`lbound > limit` or `ubound <= limit`), returns `Some(&NodeCards)`
    /// and writes `*lower_flag` accordingly. Otherwise returns `None`.
    ///
    /// Even on `None`, the lookup may have allocated a fresh block for
    /// this position (so a subsequent [`Self::add`] can extend it). This
    /// matches the vendor's `Lookup` + `Add` pairing.
    #[inline]
    pub(crate) fn lookup(
        &mut self,
        trick: i32,
        hand: i32,
        aggr_target: &[u32; TT_SUITS],
        hand_dist: &[i32; TT_HANDS],
        limit: i32,
        lower_flag: &mut bool,
    ) -> Option<&NodeCards> {
        let trick = trick as usize;
        let hand = hand as usize;
        let key = Self::suit_lengths_key(hand_dist);
        let hashkey = Self::hash8(hand_dist);

        let (block_id, empty) = self.lookup_suit(trick, hand, hashkey, key);
        self.last_block_seen[trick][hand] = Some(block_id);

        if empty {
            return None;
        }

        // Build the search pattern from the aggregator table.
        let ab0 = &self.aggr[aggr_target[0] as usize].aggr_bytes[0];
        let ab1 = &self.aggr[aggr_target[1] as usize].aggr_bytes[1];
        let ab2 = &self.aggr[aggr_target[2] as usize].aggr_bytes[2];
        let ab3 = &self.aggr[aggr_target[3] as usize].aggr_bytes[3];

        let top_set = [
            ab0[0] | ab1[0] | ab2[0] | ab3[0],
            ab0[1] | ab1[1] | ab2[1] | ab3[1],
            ab0[2] | ab1[2] | ab2[2] | ab3[2],
            ab0[3] | ab1[3] | ab2[3] | ab3[3],
        ];

        self.lookup_cards(block_id, &top_set, limit, lower_flag)
    }

    /// Find or create a `WinBlock` for the given `(trick, hand, hash, key)`.
    /// Returns `(block_id, empty)` where `empty == true` means the block
    /// is freshly allocated (nothing to match against yet).
    #[inline]
    fn lookup_suit(
        &mut self,
        trick: usize,
        hand: usize,
        hashkey: usize,
        key: i64,
    ) -> (BlockId, bool) {
        // Probe the existing entries.
        {
            let dp = &self.tt_root[trick][hand][hashkey];
            for i in 0..(dp.next_no as usize) {
                if dp.list[i].key == key {
                    return (dp.list[i].pos_block, false);
                }
            }
        }

        // Not found. Determine whether we have a free slot in the
        // hash bucket. If so, allocate a new WinBlock and bind it.
        // If the bucket is full, reuse a slot (wrap nextWriteNo).
        let n = self.tt_root[trick][hand][hashkey].next_no as usize;
        let write_no = self.tt_root[trick][hand][hashkey].next_write_no as usize;

        let (slot_idx, block_id, needs_alloc) = if n == DISTS_PER_ENTRY {
            // Bucket full — reuse existing block at `write_no`.
            let m = if write_no == DISTS_PER_ENTRY {
                0
            } else {
                write_no
            };
            let bid = self.tt_root[trick][hand][hashkey].list[m].pos_block;
            (m, bid, false)
        } else {
            // Room available — allocate a fresh block. Note: this can
            // trigger reset(), wiping everything! After reset all
            // pre-existing block references are invalid, so we have to
            // be careful about ordering.
            let bid = self.get_next_card_block();
            (n, bid, true)
        };

        // After possible reset: pull bucket again.
        let timestamp = self.timestamp;
        if needs_alloc {
            let bucket = &mut self.tt_root[trick][hand][hashkey];
            // After reset, next_no will be 0, so use that. Re-derive m.
            let n_now = bucket.next_no as usize;
            let m = if n_now == DISTS_PER_ENTRY {
                // Lost the race — bucket re-filled during the alloc.
                // Reuse current write_no.
                let w = bucket.next_write_no as usize;
                if w == DISTS_PER_ENTRY { 0 } else { w }
            } else {
                // Common path.
                bucket.next_no += 1;
                n_now
            };
            bucket.next_write_no = (m + 1) as i32;
            if bucket.next_write_no > DISTS_PER_ENTRY as i32 {
                bucket.next_write_no = 1;
            }
            bucket.list[m].pos_block = block_id;
            bucket.list[m].key = key;
            // Update the new block's timestamp_read.
            self.block_mut(block_id).timestamp_read = timestamp;
            self.block_mut(block_id).next_match_no = 0;
            self.block_mut(block_id).next_write_no = 0;
            return (block_id, true);
        }

        // n == DISTS_PER_ENTRY path: wrap or advance write pointer,
        // reuse block.
        {
            let bucket = &mut self.tt_root[trick][hand][hashkey];
            if bucket.next_write_no == DISTS_PER_ENTRY as i32 {
                bucket.next_write_no = 1;
            } else {
                bucket.next_write_no += 1;
            }
            bucket.list[slot_idx].key = key;
            bucket.list[slot_idx].pos_block = block_id;
        }
        self.block_mut(block_id).next_match_no = 0;
        self.block_mut(block_id).next_write_no = 0;
        (block_id, true)
    }

    /// Search a `WinBlock` for an entry matching `top_set` that proves
    /// the bound. Mirrors `LookupCards`.
    #[inline]
    fn lookup_cards(
        &mut self,
        block_id: BlockId,
        top_set: &[u32; TT_BYTES],
        limit: i32,
        lower_flag: &mut bool,
    ) -> Option<&NodeCards> {
        // The vendor splits the search into two loops:
        //   - First, the "recently written" half (indices nextWriteNo-1
        //     down to 0), which has most-recent entries.
        //   - Second, the "wrap-around" half (nextMatchNo-1 down to
        //     nextWriteNo), which is older but still valid until
        //     overwritten.
        let bp = self.block(block_id);
        let n = (bp.next_write_no - 1) as i32;
        let n2 = (bp.next_match_no - 1) as i32;

        // First loop: i from n down to 0.
        let mut found: Option<i32> = None;
        let mut found_lower = false;
        for i in (0..=n).rev() {
            if let Some(lower) = Self::match_entry(&bp.list[i as usize], top_set, limit) {
                found = Some(i);
                found_lower = lower;
                break;
            }
        }

        // Second loop only runs if the first didn't find anything.
        if found.is_none() {
            for i in ((n + 1)..=n2).rev() {
                if let Some(lower) = Self::match_entry(&bp.list[i as usize], top_set, limit) {
                    found = Some(i);
                    found_lower = lower;
                    break;
                }
            }
        }

        if let Some(idx) = found {
            // Bump timestamp_read, return the borrow.
            self.timestamp += 1;
            let ts = self.timestamp;
            let bp = self.block_mut(block_id);
            bp.timestamp_read = ts;
            *lower_flag = found_lower;
            Some(&self.block(block_id).list[idx as usize].first)
        } else {
            None
        }
    }

    /// One entry comparison: does `wp` match the search pattern AND
    /// prove the bound? Returns `Some(lower)` on hit, `None` on miss.
    #[inline]
    fn match_entry(wp: &WinMatch, top_set: &[u32; TT_BYTES], limit: i32) -> Option<bool> {
        if (wp.top_set[0] ^ top_set[0]) & wp.top_mask[0] != 0 {
            return None;
        }
        if wp.last_mask_no != 1 {
            if (wp.top_set[1] ^ top_set[1]) & wp.top_mask[1] != 0 {
                return None;
            }
            if wp.last_mask_no != 2 {
                if (wp.top_set[2] ^ top_set[2]) & wp.top_mask[2] != 0 {
                    return None;
                }
                // Note: vendor never checks topMask4 in lookup. That's
                // because lastMaskNo == 4 still doesn't gate further
                // checks — once we got past 3 levels, the bounds rule.
            }
        }
        let n = &wp.first;
        if n.lbound as i32 > limit {
            return Some(true);
        }
        if (n.ubound as i32) <= limit {
            return Some(false);
        }
        None
    }

    // ---- Add -------------------------------------------------------

    /// Insert/update an entry for the position whose `lookup()` was the
    /// last hash table access. Mirrors `TransTableL::Add`.
    ///
    /// `flag == false` clears the `best_move_*` fields on insert (vendor
    /// uses this when the move that produced the bound was a forced
    /// terminal — no useful hint).
    #[inline]
    pub(crate) fn add(
        &mut self,
        trick: i32,
        hand: i32,
        aggr_target: &[u32; TT_SUITS],
        win_ranks: &[u16; TT_SUITS],
        cards: NodeCards,
        lower_flag: bool,
    ) {
        let trick = trick as usize;
        let hand = hand as usize;
        let block_id = match self.last_block_seen[trick][hand] {
            Some(id) => id,
            None => return, // memory was reset since last lookup → drop
        };

        // Build the winMatchType pattern from win_ranks + aggr_target.
        let mut ab: [[u32; TT_BYTES]; TT_SUITS] = [[0; TT_BYTES]; TT_SUITS];
        let mut mb: [[u32; TT_BYTES]; TT_SUITS] = [[0; TT_BYTES]; TT_SUITS];
        let mut low: [i32; TT_SUITS] = [0; TT_SUITS];
        let mut entry = WinMatch {
            first: cards,
            xor_set: 0,
            top_set: [0; TT_BYTES],
            top_mask: [0; TT_BYTES],
            mask_index: 0,
            last_mask_no: 0,
        };

        for ss in 0..TT_SUITS {
            let w = win_ranks[ss] as i32;
            if w == 0 {
                ab[ss] = self.aggr[0].aggr_bytes[ss];
                mb[ss] = MASK_BYTES[0][ss];
                low[ss] = 15;
                entry.first.least_win[ss] = 0;
            } else {
                // Vendor: `w &= -w` extracts the lowest bit, then
                // `aggrTarget & (-w)` keeps bits >= lowest_bit. This
                // computes the aggr restricted to the "live" portion.
                let lowest_bit = w & w.wrapping_neg();
                let high_mask = lowest_bit.wrapping_neg() as u32;
                let ag = (aggr_target[ss] & high_mask) as usize & 0x1fff;
                ab[ss] = self.aggr[ag].aggr_bytes[ss];
                mb[ss] = MASK_BYTES[ag][ss];
                low[ss] = TT_LOWEST_RANK[ag];
                entry.first.least_win[ss] = (15 - low[ss]) as u8;
                entry.xor_set ^= self.aggr[ag].aggr_ranks[ss];
            }
        }

        for b in 0..TT_BYTES {
            entry.top_set[b] = ab[0][b] | ab[1][b] | ab[2][b] | ab[3][b];
            entry.top_mask[b] = mb[0][b] | mb[1][b] | mb[2][b] | mb[3][b];
        }

        entry.mask_index = (low[0] << 12) | (low[1] << 8) | (low[2] << 4) | low[3];

        entry.last_mask_no = if entry.top_mask[1] == 0 {
            1
        } else if entry.top_mask[2] == 0 {
            2
        } else if entry.top_mask[3] == 0 {
            3
        } else {
            4
        };

        self.create_or_update(block_id, &entry, lower_flag);
    }

    /// Mirror of `CreateOrUpdate`. Either tightens bounds on an existing
    /// match, or appends a new entry (wrapping if full).
    fn create_or_update(&mut self, block_id: BlockId, search: &WinMatch, flag: bool) {
        let bp = self.block_mut(block_id);
        let n = bp.next_match_no as usize;

        // Try to find an existing match to update.
        for i in 0..n {
            let wp = &bp.list[i];
            if wp.xor_set != search.xor_set {
                continue;
            }
            if wp.mask_index != search.mask_index {
                continue;
            }
            if wp.top_set[0] != search.top_set[0] {
                continue;
            }
            if wp.top_set[1] != search.top_set[1] {
                continue;
            }
            if wp.top_set[2] != search.top_set[2] {
                continue;
            }

            let dst = &mut bp.list[i].first;
            if search.first.lbound > dst.lbound {
                dst.lbound = search.first.lbound;
            }
            if search.first.ubound < dst.ubound {
                dst.ubound = search.first.ubound;
            }
            dst.best_move_suit = search.first.best_move_suit;
            dst.best_move_rank = search.first.best_move_rank;
            return;
        }

        // No existing match — append or wrap.
        if n == BLOCKS_PER_ENTRY {
            if bp.next_write_no >= BLOCKS_PER_ENTRY as i32 {
                bp.next_write_no = 0;
            }
        } else {
            bp.next_match_no += 1;
        }

        let slot = bp.next_write_no as usize;
        bp.list[slot] = *search;
        if !flag {
            bp.list[slot].first.best_move_suit = 0;
            bp.list[slot].first.best_move_rank = 0;
        }
        bp.next_write_no += 1;
    }
}

// ---------- Tests ----------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial `hand_lookup` that puts every card with North (hand 0).
    /// Useful for tests that don't care about which hand holds what.
    fn dummy_hand_lookup() -> [[i32; 15]; TT_SUITS] {
        [[0; 15]; TT_SUITS]
    }

    #[test]
    fn new_has_default_memory_limits() {
        let tt = TransTable::new();
        // Defaults are 95 and 160 MiB. Each page is ~6.3 MiB.
        // 95 * 1024 / (1000 * sizeof(WinBlock) / 1024) ≈ 14-15 pages.
        // 160 * 1024 / ... ≈ 24-25 pages.
        assert!(tt.pages_default >= 10);
        assert!(tt.pages_maximum >= tt.pages_default);
        // Maximum should be roughly the vendor's NUM_PAGES_MAXIMUM (25).
        assert!(tt.pages_maximum <= 30);
        assert!(tt.pages.is_empty());
    }

    #[test]
    fn with_memory_respects_arguments() {
        let tt = TransTable::with_memory(60, 100);
        // Same formula — verify monotonicity.
        let smaller = TransTable::with_memory(30, 50);
        assert!(tt.pages_default > smaller.pages_default);
        assert!(tt.pages_maximum > smaller.pages_maximum);
    }

    #[test]
    fn hash8_known_values() {
        // hash8 is deterministic — check a couple of fixed inputs.
        let h1 = TransTable::hash8(&[0, 0, 0, 0]);
        let h2 = TransTable::hash8(&[0x111, 0x222, 0x333, 0x444]);
        let h3 = TransTable::hash8(&[0x444, 0x333, 0x222, 0x111]);

        assert!(h1 < 256);
        assert!(h2 < 256);
        assert!(h3 < 256);
        // Different inputs produce different hashes in general.
        assert_ne!(h2, h3);
    }

    #[test]
    fn add_then_lookup_round_trip() {
        let mut tt = TransTable::new();
        tt.init(&dummy_hand_lookup());

        let trick = 5;
        let hand = 0;
        let aggr_target = [0x1fff_u32; 4]; // all cards present in each suit
        let hand_dist = [0x0433, 0x0334, 0x0232, 0x0531];
        let win_ranks = [0x1c00_u16; 4]; // top 3 cards per suit
        let cards = NodeCards {
            ubound: 9,
            lbound: 9,
            best_move_suit: 1,
            best_move_rank: 14,
            least_win: [0; 4],
        };

        // First lookup misses but registers the slot.
        let mut lf = false;
        let first = tt.lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf);
        assert!(first.is_none());

        tt.add(trick, hand, &aggr_target, &win_ranks, cards, true);

        // Now look it up — should hit since lbound (9) > limit (5).
        let mut lf2 = false;
        let _ = tt.lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf2);
        // The second lookup re-enters the bucket (since the dist matches)
        // and should match the entry we just added.
        // We don't assert on the borrow here because we need to do
        // multiple lookups; instead assert via copy.
        let res = tt
            .lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf2)
            .copied();
        assert!(res.is_some(), "lookup after add must return the entry");
        let got = res.unwrap();
        assert_eq!(got.ubound, 9);
        assert_eq!(got.lbound, 9);
        assert_eq!(got.best_move_suit, 1);
        assert_eq!(got.best_move_rank, 14);
        assert!(lf2, "lbound > limit, so lower_flag must be set");
    }

    #[test]
    fn lookup_with_different_hand_dist_misses() {
        let mut tt = TransTable::new();
        tt.init(&dummy_hand_lookup());

        let trick = 7;
        let hand = 1;
        let aggr_target = [0x1fff_u32; 4];
        let hd_a = [0x0433, 0x0334, 0x0232, 0x0531];
        let hd_b = [0x0532, 0x0334, 0x0232, 0x0431]; // different
        let win_ranks = [0x1c00_u16; 4];
        let cards = NodeCards {
            ubound: 10,
            lbound: 10,
            best_move_suit: 2,
            best_move_rank: 11,
            least_win: [0; 4],
        };

        // Add under hd_a.
        let mut lf = false;
        let _ = tt.lookup(trick, hand, &aggr_target, &hd_a, 5, &mut lf);
        tt.add(trick, hand, &aggr_target, &win_ranks, cards, true);

        // Lookup under hd_a: hit.
        let mut lf = false;
        let _ = tt.lookup(trick, hand, &aggr_target, &hd_a, 5, &mut lf);
        let res_a = tt
            .lookup(trick, hand, &aggr_target, &hd_a, 5, &mut lf)
            .copied();
        assert!(res_a.is_some());

        // Lookup under hd_b: distinct distribution, should miss.
        let mut lf = false;
        let res_b = tt
            .lookup(trick, hand, &aggr_target, &hd_b, 5, &mut lf)
            .copied();
        assert!(
            res_b.is_none(),
            "different hand_dist must not return entry from hd_a"
        );
    }

    #[test]
    fn add_two_entries_with_different_dists_both_retrievable() {
        let mut tt = TransTable::new();
        tt.init(&dummy_hand_lookup());

        let trick = 8;
        let hand = 2;
        let aggr_target = [0x1fff_u32; 4];
        let hd_a = [0x0433, 0x0334, 0x0232, 0x0531];
        let hd_b = [0x0532, 0x0334, 0x0232, 0x0431];
        let win_ranks = [0x1c00_u16; 4];

        let mut lf = false;
        let _ = tt.lookup(trick, hand, &aggr_target, &hd_a, 5, &mut lf);
        tt.add(
            trick,
            hand,
            &aggr_target,
            &win_ranks,
            NodeCards {
                ubound: 7,
                lbound: 7,
                best_move_suit: 0,
                best_move_rank: 5,
                least_win: [0; 4],
            },
            true,
        );

        let _ = tt.lookup(trick, hand, &aggr_target, &hd_b, 5, &mut lf);
        tt.add(
            trick,
            hand,
            &aggr_target,
            &win_ranks,
            NodeCards {
                ubound: 8,
                lbound: 8,
                best_move_suit: 1,
                best_move_rank: 6,
                least_win: [0; 4],
            },
            true,
        );

        let mut lf = false;
        let _ = tt.lookup(trick, hand, &aggr_target, &hd_a, 3, &mut lf);
        let got_a = tt
            .lookup(trick, hand, &aggr_target, &hd_a, 3, &mut lf)
            .copied();
        assert_eq!(got_a.map(|n| n.best_move_rank), Some(5));

        let _ = tt.lookup(trick, hand, &aggr_target, &hd_b, 3, &mut lf);
        let got_b = tt
            .lookup(trick, hand, &aggr_target, &hd_b, 3, &mut lf)
            .copied();
        assert_eq!(got_b.map(|n| n.best_move_rank), Some(6));
    }

    #[test]
    fn reset_clears_all_entries() {
        let mut tt = TransTable::new();
        tt.init(&dummy_hand_lookup());

        let trick = 6;
        let hand = 3;
        let aggr_target = [0x1fff_u32; 4];
        let hand_dist = [0x0433, 0x0334, 0x0232, 0x0531];
        let win_ranks = [0x1c00_u16; 4];

        let mut lf = false;
        let _ = tt.lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf);
        tt.add(
            trick,
            hand,
            &aggr_target,
            &win_ranks,
            NodeCards {
                ubound: 10,
                lbound: 10,
                best_move_suit: 0,
                best_move_rank: 12,
                least_win: [0; 4],
            },
            true,
        );

        // Should be present.
        let _ = tt.lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf);
        let res = tt
            .lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf)
            .copied();
        assert!(res.is_some(), "pre-reset lookup must hit");

        tt.reset();

        // After reset, last_block_seen is None, no entries exist.
        assert!(tt.last_block_seen[trick as usize][hand as usize].is_none());

        // First lookup post-reset: must miss (no entries).
        let mut lf = false;
        let res = tt
            .lookup(trick, hand, &aggr_target, &hand_dist, 5, &mut lf)
            .copied();
        assert!(res.is_none(), "post-reset lookup must miss");
    }

    #[test]
    fn lookup_without_init_returns_none() {
        // Without `init`, the aggr table is all zeros — so all positions
        // hash identically, but no entries exist. First lookup misses,
        // and subsequent add stores under the (uninformative) all-zero
        // pattern.
        let mut tt = TransTable::new();
        let mut lf = false;
        let res = tt.lookup(0, 0, &[0; 4], &[0; 4], 0, &mut lf);
        assert!(res.is_none());
    }

    #[test]
    fn set_memory_changes_budget() {
        let mut tt = TransTable::new();
        let original_default = tt.pages_default;
        let original_max = tt.pages_maximum;

        tt.set_memory_default(40);
        tt.set_memory_maximum(80);

        assert_ne!(tt.pages_default, original_default);
        assert_ne!(tt.pages_maximum, original_max);
        assert!(tt.pages_default < original_default);
        assert!(tt.pages_maximum < original_max);
    }

    #[test]
    fn page_allocation_grows_pool() {
        let mut tt = TransTable::with_memory(10, 20);
        tt.init(&dummy_hand_lookup());
        // Force allocation by inserting many distinct distributions.
        let aggr_target = [0x1fff_u32; 4];
        let win_ranks = [0x1c00_u16; 4];
        let cards = NodeCards {
            ubound: 5,
            lbound: 5,
            best_move_suit: 0,
            best_move_rank: 5,
            least_win: [0; 4],
        };
        let initial_pages = tt.pages.len();

        // Insert a few hundred distinct positions.
        for i in 0..500i32 {
            let hd = [i & 0xfff, (i * 2) & 0xfff, (i * 3) & 0xfff, (i * 5) & 0xfff];
            let mut lf = false;
            let _ = tt.lookup(5, 0, &aggr_target, &hd, 3, &mut lf);
            tt.add(5, 0, &aggr_target, &win_ranks, cards, true);
        }
        assert!(tt.pages.len() > initial_pages);
    }
}
