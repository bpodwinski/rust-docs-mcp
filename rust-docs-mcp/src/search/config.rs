//! # Search Configuration Module
//!
//! Provides configuration constants for search indexing and querying.
//!
//! These constants control resource usage and performance characteristics
//! of the search functionality.

/// Default buffer size for the Tantivy index writer (50MB).
///
/// Tantivy splits this budget across up to 8 indexing threads, with a
/// per-thread floor of ~15MB. 50MB yields roughly 3 indexing threads —
/// which benchmarks show is actually a sweet spot for the small-to-medium
/// crates that make up the common path. Raising this to 256MB (hoping to
/// enable full 8-way parallelism) measured as a ~2x time regression on
/// a small crate like `tantivy`, because tantivy's coordination overhead
/// with 8 threads on a few-thousand-item workload dominates the parallel
/// speedup. Very large crates (several hundred thousand items) may
/// benefit from a larger budget, but we'd want to size it dynamically
/// based on `crate.index.len()` rather than statically.
pub const DEFAULT_BUFFER_SIZE: usize = 50_000_000;

/// Maximum number of items to index per crate.
///
/// This is a pathological-crate safety net, not a routine cap. Large
/// crates like `windows-rs` have several hundred thousand items; setting
/// this to 1M leaves comfortable headroom while still rejecting runaway
/// macro-generated output.
pub const MAX_ITEMS_PER_CRATE: usize = 1_000_000;

/// Default limit for search results
pub const DEFAULT_SEARCH_LIMIT: usize = 50;

/// Maximum allowed limit for search results
pub const MAX_SEARCH_LIMIT: usize = 1000;

/// Maximum allowed query length in characters
pub const MAX_QUERY_LENGTH: usize = 1000;

/// Default fuzzy distance for typo tolerance
pub const DEFAULT_FUZZY_DISTANCE: u8 = 1;

/// Maximum fuzzy distance allowed
pub const MAX_FUZZY_DISTANCE: u8 = 2;

/// Whether transpositions cost 1 edit instead of 2 in fuzzy matching
/// This makes fuzzy search more forgiving for common typos like "teh" -> "the"
pub const FUZZY_TRANSPOSE_COST_ONE: bool = true;
