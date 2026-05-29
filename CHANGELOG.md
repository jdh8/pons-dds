# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog.

## [Unreleased]

## [0.1.0] - 2026-05-30

Initial release of pons-dds, a pure-Rust double dummy solver for contract bridge.

### Added

- Public solver entry points `solve_deal` and `solve_deals`.
- Reusable `Solver` and `solve_deal_on` path for sequential/diagnostic usage.
- Criterion benchmark suite in `benches/solver.rs` covering `solve_deal` and `solve_deals/{32,200}`.
- GitHub Actions benchmark publication to GitHub Pages benchmark dashboard (`dev/bench`) via `github-action-benchmark`.
- README badges for CI, Crates.io, Docs.rs, and published benchmarks.

### Notes

- Benchmarks are published continuously from `main` and intended for trend/regression tracking across commits.
