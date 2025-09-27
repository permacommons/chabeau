# Benchmarks (Criterion 0.7)

This repository ships a `render_cache` bench to validate the cached prewrapped rendering path. Use this scaffold to add your own when validating performance-sensitive changes.

## Steps

1) Ensure Criterion is available (already in `Cargo.toml`):

```
[dev-dependencies]
criterion = "0.7"
```

2) If you need to import internal modules (e.g., for UI/scroll perf): temporarily add a `src/lib.rs` that re-exports the modules you need, for example:

```
// src/lib.rs (temporary for benches)
pub mod api;
pub mod auth;
pub mod commands;
pub mod core;
pub mod ui;
pub mod utils;
```

3) Create a bench file under `benches/`, e.g., `benches/my_bench.rs`:

```
use criterion::{criterion_group, criterion_main, Criterion};

// Example: import internal items via the temporary lib target
// use chabeau::utils::scroll::ScrollCalculator;

fn bench_example(c: &mut Criterion) {
    c.bench_function("example", |b| {
        b.iter(|| {
            // put benchmarked code here
        })
    });
}

criterion_group!(benches, bench_example);
criterion_main!(benches);
```

4) Run benches (includes `render_cache`):

```
cargo bench --features bench
```

5) Reports are written under `target/criterion/` (open `report/index.html`).

6) The library target (`src/lib.rs`) is checked in so benches can import internal modules.

## Notes

- Prefer small, focused benches that isolate the hot paths you changed.
- Keep benches deterministic (avoid network or filesystem outside the workspace).
- If benches are useful long-term, consider checking them in and updating README accordingly.
