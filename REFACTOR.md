# Boolean → score refactor for `ab_search_*`

## Why

The bisection driver in `Engine::search_target` calls `ab_search_0` an
average of **3.86 times per invocation**. Per-iter wall-clock timing
shows iters 2..N are **2× more expensive** than iter 1 (1.26 ms vs
2.58 ms avg), and consume 85% of total search time. Iter 1 is cheap
because the midpoint target is the "easy" question that `quick_tricks`
typically proves in one shot; iters 2..N nail down the tight boundary
near the true score where pruning is least effective.

If `ab_search_0` returned an **exact score** instead of a boolean,
`search_target` would call it **once**. The work would be roughly
equivalent to the single hardest probe (~2.5–3.5 ms) instead of the
sum of 1.26 + 2.86×2.58 = **8.6 ms**. End-to-end estimated speedup:
**50–60%** on the `ab_search_*` bucket, which is ~42% of total CPU
self-time. Net deal throughput: **~25–30% faster**.

Per-iter timing instrumentation lives in `Engine::iter1_nanos` /
`later_nanos`; the `bisection_stats` example reports the ratio. Use it
to validate progress.

## Goal

Replace the null-window boolean alpha-beta with full alpha-beta that
returns the exact minimax score. Keep the recursive structure
(`ab_search_0/1/2/3`), the TT layout, and the `quick_tricks` /
`later_tricks` helpers — only their signatures and return semantics
change.

End state:
- `ab_search_*(pos, tt, alpha, beta, depth) -> i32` returning the score
  (in tricks won by MAX), clipped per fail-high / fail-low rules.
- `Engine::search_target` becomes a single call with the wide window
  `[0, max_tricks + 1)`. The bisection loop and `bisection_iters`
  counter remain only as instrumentation (will read as `≈ 1.0` after
  the refactor).
- `TransTable::add` / `tt_lookup` switched to score-based bounds
  derivation. The on-disk `NodeCards` layout (`lbound`, `ubound`,
  `best_move_*`, `least_win`) is unchanged.

## Architectural translation

Current null-window boolean call:
```
ab_search_0(pos, tt, target, depth) -> bool
  // true iff MAX achieves ≥ target tricks from this subtree
```

Refactor:
```
ab_search_0(pos, tt, alpha, beta, depth) -> i32
  // returns v such that:
  //   v ≥ beta  → lower bound (failed high, real score ≥ v)
  //   v ≤ alpha → upper bound (failed low, real score ≤ v)
  //   else      → exact score
```

The boolean `ab_search_*(target)` is equivalent to the score call
`ab_search_*(target - 1, target) ≥ target`.

## Step-by-step plan

Order matters — each step compiles and tests pass on its own. The
cross-check tests at `tests/cross_check.rs` against `dds-bridge` and
`ddss` are the correctness safety net at every step.

### Step 1: convert `quick_tricks` and `later_tricks_*` to bound returns

`src/quick_tricks.rs:74-250` (`quick_tricks`) currently returns
`(qtricks, *success)` where `success` says whether `qtricks` proves
the boolean against the caller's target.

Change to return `(lower_bound, exact_p)` — `lower_bound` is the
number of tricks MAX is *guaranteed* to take from this subtree;
`exact_p` is true when no further search is needed (the value is the
final answer for this side, e.g., MIN side has no playable resistance).

`src/later_tricks.rs:later_tricks_min` and `later_tricks_max` —
similar. Currently return `bool` against a target. New contract:
return an `Option<i32>` that is `Some(upper_bound_on_max_score)` when
the lookup is decisive, `None` when search must continue.

These two files are leaves of the refactor — no recursion into search,
so they can be converted first without breaking `ab_search_*`. Update
the call sites in `search.rs` to bridge back to the old boolean for
now (compare bound to target). Confirm tests pass.

### Step 2: define score-returning `ab_search_*` signatures

In `src/search.rs`, add new methods:
```rust
fn ab_score_0(&mut self, pos: &mut Pos, tt: &mut TransTable,
              alpha: i32, beta: i32, depth: i32) -> i32
// same for _1, _2, _3
```

Initial body: wrap the existing boolean function. For step 2, this is
just a shim — `ab_score_0(a, b)` calls `ab_search_0(b)` and returns
`b` if true, `a` if false. Compiles, but no behavioral change.

Goal of this step: the score-returning surface exists and the project
still passes tests.

### Step 3: rewrite `ab_search_3` body in score form

`ab_search_3` (`src/search.rs:853-930`) is the simplest because it
just resolves the trick and recurses into `ab_search_0`. The
recursive child returns a score; no minimax min/max happens at this
level beyond move-loop alpha-beta cutoff.

Translation rules for the move loop:
```rust
// boolean
let mut value = !success;
for move {
    value = ab_search_0(target, depth-1);
    if value == success { cutoff; break; }
}
```
becomes
```rust
let mut best = if is_max { i32::MIN } else { i32::MAX };
let mut a = alpha; let mut b = beta;
for move {
    let v = ab_score_0(a, b, depth-1);
    if is_max {
        if v > best { best = v; chosen = move; }
        a = a.max(best);
        if a >= b { cutoff; break; }   // β-cutoff
    } else {
        if v < best { best = v; chosen = move; }
        b = b.min(best);
        if a >= b { cutoff; break; }   // α-cutoff
    }
}
```

`pos.tricks_max += 1` / `-= 1` around the recursive call: keep
unchanged. The child's returned score is the score from THIS position
too (it includes the trick we just won).

After this step, `ab_search_3` is score-native but `ab_search_0/1/2`
are still booleans (shim from step 2 bridges back). Tests must pass.

### Step 4: rewrite `ab_search_2` and `ab_search_1`

Mechanical translation of the same move-loop pattern. The `success`
local goes away — replaced by `is_max = node_type_store[hand_u] == MAXNODE`.

### Step 5: rewrite `ab_search_0`

The hairy one. Handle in this order:

- **Quick eval checks** (`src/search.rs:545-549`):
  ```rust
  if pos.tricks_max >= target { return true; }
  if pos.tricks_max + tricks + 1 < target { return false; }
  ```
  become score cutoffs:
  ```rust
  if pos.tricks_max >= beta { return pos.tricks_max; }            // β-cutoff
  if pos.tricks_max + tricks + 1 <= alpha {
      return pos.tricks_max + tricks + 1;                         // α-cutoff
  }
  ```
- **Leaf** (`depth == 0`): `evaluate` already returns the exact score.
  Return it directly.
- **Quick tricks integration**: use the lower bound from the new
  `quick_tricks` contract to update `alpha` (if MAX side) or for an
  immediate β-cutoff. Symmetric for MIN with `later_tricks_max` upper
  bound.
- **TT lookup**: returns `Option<TTHit>` where `TTHit` is either an
  exact score, a lower bound, or an upper bound. Use to tighten
  alpha/beta or return outright.
- **Move loop**: same pattern as `ab_search_3`.
- **TT store**: derive bound type from the relationship of `best` to
  the *original* `alpha` / `beta`:
  - `best ≥ orig_beta` → store lbound = best (fail high)
  - `best ≤ orig_alpha` → store ubound = best (fail low)
  - else → store lbound = ubound = best (exact)

### Step 6: replace the bisection driver

`Engine::search_target` (`src/search.rs:939-984`) becomes:
```rust
pub(crate) fn search_target(&mut self, pos, tt, ini_depth) -> i32 {
    self.search_target_calls += 1;
    self.ini_depth = ini_depth;

    // ... edge case / init unchanged ...

    let max = ((ini_depth + 4) >> 2).max(0);
    self.bisection_iters += 1;          // for instrumentation parity
    let t0 = std::time::Instant::now();
    let score = self.ab_score_0(pos, tt, 0, max + 1, ini_depth);
    self.iter1_nanos += t0.elapsed().as_nanos();
    score
}
```

After this step, `bisection_stats.rs` should report `avg iters per call: 1.000`.

### Step 7: remove the old boolean `ab_search_*`

Once all call sites use `ab_score_*`, delete the boolean functions and
their bridging logic. Drop the `success` / `bool` plumbing from
`ab_search_3`'s `value == success` comparison.

## TT entry layout

`NodeCards` (`src/tt.rs:63-72`) is already a score-based bound store
(`lbound: i8`, `ubound: i8`). No layout change needed — only the
*derivation* of these fields at store time, and the *use* of them at
lookup time, are altered.

Current `tt_lookup` (`src/search.rs:672-712`) computes a `limit`
relative to `target` and `pos.tricks_max` then compares against
`lower_flag`. Replace with direct bound consultation:

```rust
fn tt_lookup_score(...) -> Option<TTHit> {
    // ... existing match/find ...
    let lb = cards.lbound as i32 + pos.tricks_max;
    let ub = cards.ubound as i32 + pos.tricks_max;
    if lb >= beta  { return Some(TTHit::FailHigh(lb)); }
    if ub <= alpha { return Some(TTHit::FailLow(ub)); }
    if lb == ub    { return Some(TTHit::Exact(lb)); }
    Some(TTHit::Bounds { lb, ub })
}
```

Verify the `+ pos.tricks_max` offset by comparing to the existing
boolean lookup — the stored bounds in the current code are also
relative; the offset needs to match.

## Test strategy

- After each step, `cargo test --all-features` must pass — especially
  `tests/cross_check.rs` (~60s) which validates against `dds-bridge` and
  `ddss` on random deals.
- Re-run `cargo run --release --example bisection_stats -- 200` after
  steps 5, 6, and 7. Expected progression:
  - Steps 1–5: ms/deal flat (or slightly slower from extra branches in
    shim layer), `avg iters per call` still ~3.86.
  - Step 6: `avg iters per call` → 1.000, ms/deal drops sharply.
- Capture a final flamegraph (`cargo flamegraph --bench solver -- --bench '^solve_deal_single$'`)
  and confirm the `ab_search_*` self-time bucket shrinks proportionally.

## Risks and pitfalls

1. **Silent slowdowns from missing α-β cutoffs.** Cross-check tests
   only verify final trick counts. A bug that disables a cutoff
   produces correct results, slower — only the benchmark catches it.
   Mitigation: after step 6, ms/deal MUST drop. If it doesn't, a
   cutoff is missing.
2. **`tricks_max` offset on TT bounds.** The current code stores
   bounds relative to `tricks_max` at search time, and the lookup
   accounts for the difference. Mishandling this gives wrong results
   (cross-check will catch — but the diagnosis is non-obvious).
3. **`quick_tricks` early-exact return.** When `quick_tricks` decides
   the position is fully determined (current `*success = true` path),
   the returned `qtricks` is the EXACT subtree value. Store as
   `lbound = ubound = qtricks + pos.tricks_max` in the TT so future
   queries cache-hit immediately.
4. **`win_ranks` propagation.** Independent of the boolean→score
   change; preserve the existing merge pattern from commit `b6bdf9f`.
5. **Move ordering relies on `best_move` from cutoff.** Keep updating
   `best_move[depth_u]` exactly when an α-β cutoff fires — don't drop
   this in the rewrite.

## Validation checklist

- [ ] `cargo test --all-features` — all 50 tests pass after each step
- [ ] `cargo clippy --all-targets -- -W clippy::nursery -W clippy::pedantic` — clean
- [ ] `cargo run --release --example bisection_stats -- 200`:
  - `avg iters per call` → `1.000`
  - `ms per deal` drops to ~70–90 ms (vs 177 baseline)
  - `iter 1` accounts for ~100% of search time (no iters 2..N)
- [ ] Flamegraph: `ab_search_*` self-time bucket halves or better
- [ ] `cargo bench --bench solver` shows ≥ 20% throughput improvement

## File-by-file impact

| File | What changes | Approx LOC delta |
|---|---|---|
| `src/search.rs` | All 4 `ab_search_*` bodies + driver + TT lookup | ~+200 / −150 |
| `src/tt.rs` | `tt_lookup`/`add` semantics; new `TTHit` enum (optional) | ~+30 / −10 |
| `src/quick_tricks.rs` | Return signature change | ~+10 / −5 |
| `src/later_tricks.rs` | Return signature change | ~+10 / −5 |
| `examples/bisection_stats.rs` | Update interpretation thresholds | ~+5 |

## Time estimate

Focused work: **2–3 days**. Step 1 (~half day, mechanical). Steps 2–4
(~half day, structural). Step 5 (~one day, requires care). Steps 6–7
(~half day, including measurement). Cross-check correctness debugging
buffer: ~half day.
