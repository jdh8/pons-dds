# pons-dds-web

A browser demo that runs [pons-dds](..) — a pure-Rust double-dummy solver —
entirely client-side as WebAssembly.  There is no server: the deal is solved in
your browser.

Two tabs:

- **Solve** — deal a random hand (or import one from **Edit**) and see the full
  5×4 double-dummy trick table, the [par](../src/par.rs) contract and score, and
  the best opening lead against that par contract.
- **Edit** — a PBN text field two-way-synced with a 4×13 card palette; click a
  card to cycle its owner N→E→S→W→out, then **Solve →**.

The solver is driven strictly on its single-threaded paths (`Solver`,
`solve_deal_on`, `Solver::solve_board`); the rayon fan-out in the free `solve_*`
functions would need threads wasm doesn't have.  Random dealing and all hand
rendering are plain JS, so the only thing that crosses into wasm is the solve.

## Build

You need the `wasm32-unknown-unknown` target and `wasm-pack`:

```console
rustup target add wasm32-unknown-unknown   # once
cargo install wasm-pack                    # once

wasm-pack build --release --target web     # writes ./pkg/
```

Notes:

- `.cargo/config.toml` clears a global `-Ctarget-cpu=native` for the wasm build
  (left in place it corrupts the module's target features and `wasm-bindgen`
  then fails with `failed to find intrinsics to enable "clone_ref"`) and gives
  the linker a 16 MiB stack — a double-dummy solve recurses ~50 plies deep and
  overflows wasm's default 1 MiB shadow stack as silent memory corruption.
- No `getrandom`: random deals are shuffled in JS, so nothing here needs a
  wasm RNG backend.

## Run

Serve this directory over HTTP — ES modules and wasm won't load from `file://`:

```console
python3 -m http.server 8137
# open http://localhost:8137/
```

## Test

The wasm surface is native-testable without a browser:

```console
cargo test
```

## Deploy

`pkg/`, `index.html`, `app.js`, and `style.css` are all static — push them to
GitHub Pages (`../.github/workflows/pages.yml` does exactly this) or any static
host.
