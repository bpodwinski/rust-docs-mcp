//! Benchmark for the search indexing pipeline against a large rustdoc JSON.
//!
//! Measures:
//! - Wall-clock indexing time (via criterion)
//! - Peak heap usage (via `peak_alloc`, reported to stderr per iteration)
//!
//! Fixture handling:
//! - Target crate is cached under `$HOME/.cache/rust-docs-mcp-bench/` by
//!   default. This has to live OUTSIDE the rust-docs-mcp workspace: if the
//!   fixture sits under `target/`, `cargo rustdoc` invoked inside a
//!   downloaded crate walks up and finds our own workspace manifest, and
//!   cargo refuses to build. Override with `BENCH_CACHE_DIR=/some/path`.
//! - Crate name + version come from `BENCH_CRATE` / `BENCH_VERSION`;
//!   defaults to `windows = "0.58.0"`.
//! - The fixture (source + `docs.json`) is generated once on first run.
//!   After that, each iteration only re-runs parse + tantivy indexing.
//! - Per-iteration: the tantivy index directory is wiped in the setup
//!   closure so we measure a clean re-index each time; the peak allocator
//!   counter is reset in the same closure so only the measured body
//!   contributes to the reported peak.
//! - To reclaim disk: `rm -rf ~/.cache/rust-docs-mcp-bench`.
//!
//! Baseline workflow:
//! - `cargo bench --bench indexing -- --save-baseline pre_fix` — record a pre-fix baseline
//! - apply fix
//! - `cargo bench --bench indexing -- --baseline pre_fix` — criterion prints deltas
//!   against the saved baseline; stderr still shows per-iteration heap peaks.
//!
//! NOTE: Pre-fix code has a hard cap of `MAX_ITEMS_PER_CRATE = 100_000` which
//! `windows` will exceed. To establish a baseline on pre-fix code, temporarily
//! raise that cap on the baseline commit so the bench can run.

use std::path::PathBuf;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use peak_alloc::PeakAlloc;
use rust_docs_mcp::cache::service::CrateCache;
use rust_docs_mcp::cache::storage::CacheStorage;
use tokio::runtime::Runtime;

#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

fn bench_cache_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("BENCH_CACHE_DIR") {
        return PathBuf::from(custom);
    }
    // Default: $HOME/.cache/rust-docs-mcp-bench/. Must live outside any
    // cargo workspace so downloaded crates don't pick up our manifest.
    dirs::home_dir()
        .expect("$HOME is set")
        .join(".cache")
        .join("rust-docs-mcp-bench")
}

/// `std::fs::remove_dir_all` is flaky on macOS when a directory has just had
/// files closed: it can return `ENOTEMPTY` ("Directory not empty") even
/// though nothing else is holding the handles. Retry a few times with a
/// short sleep before giving up. See rust-lang/rust#60025 for background.
fn wipe_dir_with_retry(path: &std::path::Path) -> std::io::Result<()> {
    use std::thread;
    use std::time::Duration;
    let mut last_err = None;
    for attempt in 0..5 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(50 * (attempt + 1)));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("wipe_dir_with_retry exhausted")))
}

fn bench_indexing(c: &mut Criterion) {
    let name = std::env::var("BENCH_CRATE").unwrap_or_else(|_| "windows".to_string());
    let version = std::env::var("BENCH_VERSION").unwrap_or_else(|_| "0.58.0".to_string());
    let cache_dir = bench_cache_dir();

    eprintln!(
        "=== indexing bench ===\n  crate:     {name}-{version}\n  cache_dir: {}",
        cache_dir.display()
    );

    // Shared tokio runtime — criterion itself is sync, we just block_on inside
    // each iteration.
    let runtime = Runtime::new().expect("tokio runtime");
    let cache = CrateCache::new(Some(cache_dir.clone())).expect("cache init");
    // Separate CacheStorage for path computation (the one inside CrateCache is
    // crate-private). Both point at the same directory so path resolution is
    // identical.
    let storage = CacheStorage::new(Some(cache_dir.clone())).expect("storage init");

    // One-time fixture setup: download + generate docs.json if not already cached.
    // This is expensive (minutes) but runs once per `cargo clean`.
    let docs_path = storage
        .docs_path(&name, &version, None)
        .expect("docs_path");
    if !docs_path.exists() {
        eprintln!("Fixture not found; generating (this may take several minutes)...");
        runtime.block_on(async {
            cache
                .ensure_crate_docs(&name, &version, None)
                .await
                .expect("ensure_crate_docs");
        });
        eprintln!("Fixture generated at {}", docs_path.display());
    } else {
        eprintln!("Reusing cached fixture at {}", docs_path.display());
    }

    // Report fixture size for context.
    if let Ok(meta) = std::fs::metadata(&docs_path) {
        let mb = (meta.len() as f64) / (1024.0 * 1024.0);
        eprintln!("docs.json size: {mb:.1} MB");
    }

    let mut group = c.benchmark_group("indexing");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(180));
    group.warm_up_time(Duration::from_secs(5));

    let mut iter_counter = 0usize;
    let bench_id = format!("{name}-{version}");

    group.bench_function(bench_id, |b| {
        b.iter_batched(
            || {
                // Setup: wipe any existing tantivy index so every measured call
                // builds from scratch, then reset the peak allocator counter so
                // the reported peak only reflects the measured body.
                let index_path = storage
                    .search_index_path(&name, &version, None)
                    .expect("search_index_path");
                if index_path.exists() {
                    wipe_dir_with_retry(&index_path).expect("wipe old index");
                }
                PEAK_ALLOC.reset_peak_usage();
            },
            |()| {
                runtime.block_on(async {
                    cache
                        .create_search_index(&name, &version, None)
                        .await
                        .expect("create_search_index");
                });
                iter_counter += 1;
                let peak_mb = PEAK_ALLOC.peak_usage_as_mb();
                eprintln!("iter {iter_counter:>3}: peak heap = {peak_mb:>8.1} MB");
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_indexing);
criterion_main!(benches);
