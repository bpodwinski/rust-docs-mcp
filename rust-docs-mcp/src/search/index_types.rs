//! Lightweight types for the search indexing pipeline.
//!
//! [`IndexCrate`] and [`IndexItem`] mirror the shape of
//! [`rustdoc_types::Crate`] / [`rustdoc_types::Item`] but skip every field
//! the indexer doesn't read — most importantly the deeply recursive
//! [`rustdoc_types::ItemEnum`] subtree.  The `inner` field is replaced by a
//! `kind_tag: &'static str` that captures only the enum discriminant (e.g.
//! `"function"`, `"struct"`) via a custom serde `Deserialize` that calls
//! [`serde::de::IgnoredAny`] to skip the value without allocating.
//!
//! This drops the indexing-path peak heap from ~2× docs.json to ~0.3–0.5×
//! because the per-item `Function.sig`, `Impl.trait_`, `Generics.params`,
//! etc. are never materialised.

use rustdoc_types::{Id, ItemSummary, Visibility};
use serde::Deserialize;
use std::collections::HashMap;

/// Trimmed crate representation for the indexing-only path.
///
/// Deserializes the same rustdoc JSON as [`rustdoc_types::Crate`] but only
/// keeps the two maps the indexer iterates (`index` and `paths`).  All other
/// top-level fields (`root`, `crate_version`, `includes_private`,
/// `external_crates`, `target`, `format_version`) are silently skipped by
/// serde's default behaviour for structs.
#[derive(Deserialize)]
pub struct IndexCrate {
    pub index: HashMap<Id, IndexItem>,
    pub paths: HashMap<Id, ItemSummary>,
}

/// Lightweight stand-in for [`rustdoc_types::Item`].
///
/// Keeps only the fields the search indexer actually reads:
///
/// | Field | Used by |
/// |-------|---------|
/// | `name` | document name field |
/// | `docs` | full-text search body |
/// | `visibility` | stored metadata |
/// | `kind_tag` (from `inner`) | kind facet (e.g. `"function"`) |
///
/// The expensive `inner` subtree (`ItemEnum`) is replaced by
/// [`kind_tag`](Self::kind_tag), which is deserialised with a custom
/// visitor that reads only the externally-tagged variant key and skips the
/// value via [`serde::de::IgnoredAny`].
#[derive(Deserialize)]
pub struct IndexItem {
    pub name: Option<String>,
    pub docs: Option<String>,
    pub visibility: Visibility,
    /// The `ItemEnum` variant tag (e.g. `"function"`, `"struct"`).
    ///
    /// Stored as `String` rather than `&'static str` to keep the
    /// `#[derive(Deserialize)]` compatible with serde's lifetime
    /// inference. The allocation is negligible (~10 bytes) compared
    /// to the kilobytes of `ItemEnum` subtree we skip per item.
    #[serde(deserialize_with = "deserialize_kind_tag", rename = "inner")]
    pub kind_tag: String,
}

// ---------------------------------------------------------------------------
// Custom serde: extract only the ItemEnum variant tag from `inner`
// ---------------------------------------------------------------------------

/// Map a JSON variant-tag string to the same `&'static str` that
/// [`crate::docs::query::item_kind_str`] returns for the corresponding
/// [`rustdoc_types::ItemEnum`] variant.
///
/// Uses `_ => "unknown"` so that new variants added in future
/// `rustdoc-types` releases don't cause deserialization failures.
fn tag_to_kind(tag: &str) -> String {
    match tag {
        "module" => "module",
        "struct" => "struct",
        "enum" => "enum",
        "function" => "function",
        "trait" => "trait",
        "impl" => "impl",
        "type_alias" => "type_alias",
        "constant" => "constant",
        "static" => "static",
        "macro" => "macro",
        "extern_crate" => "extern_crate",
        "use" => "use",
        "union" => "union",
        // `ItemEnum::StructField` → snake_case is `struct_field`, but
        // `item_kind_str` returns `"field"`.
        "struct_field" => "field",
        "variant" => "variant",
        "trait_alias" => "trait_alias",
        "proc_macro" => "proc_macro",
        "primitive" => "primitive",
        "assoc_const" => "assoc_const",
        "assoc_type" => "assoc_type",
        "extern_type" => "extern_type",
        _ => "unknown",
    }
    .to_string()
}

/// Deserialise the `inner` field of an Item as just the variant tag string.
///
/// Rustdoc JSON uses serde's default externally-tagged enum encoding:
///
/// - **Newtype/struct variants** (most): `{"function": { ... }}`
///   → handled by `visit_map`: read the single key, skip the value via
///   [`serde::de::IgnoredAny`].
/// - **Unit variants** (`ExternType`): `"extern_type"`
///   → handled by `visit_str`: return the tag directly.
fn deserialize_kind_tag<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct KindTagVisitor;

    impl<'de> serde::de::Visitor<'de> for KindTagVisitor {
        type Value = String;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an externally-tagged ItemEnum variant (map or string)")
        }

        // Unit variant: `"extern_type"`
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<String, E> {
            Ok(tag_to_kind(v))
        }

        // Newtype/struct variant: `{"function": { ... }}`
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<String, A::Error> {
            let key: String = map
                .next_key()?
                .ok_or_else(|| serde::de::Error::custom("empty map for ItemEnum inner"))?;
            // Skip the entire value subtree without allocating.
            map.next_value::<serde::de::IgnoredAny>()?;
            Ok(tag_to_kind(&key))
        }
    }

    deserializer.deserialize_any(KindTagVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that every known `ItemEnum` variant tag maps to the expected
    /// kind string, matching what `item_kind_str` would return.
    #[test]
    fn tag_to_kind_covers_all_variants() {
        let cases = [
            ("module", "module"),
            ("struct", "struct"),
            ("enum", "enum"),
            ("function", "function"),
            ("trait", "trait"),
            ("impl", "impl"),
            ("type_alias", "type_alias"),
            ("constant", "constant"),
            ("static", "static"),
            ("macro", "macro"),
            ("extern_crate", "extern_crate"),
            ("use", "use"),
            ("union", "union"),
            ("struct_field", "field"),
            ("variant", "variant"),
            ("trait_alias", "trait_alias"),
            ("proc_macro", "proc_macro"),
            ("primitive", "primitive"),
            ("assoc_const", "assoc_const"),
            ("assoc_type", "assoc_type"),
            ("extern_type", "extern_type"),
        ];
        for (tag, expected) in cases {
            assert_eq!(tag_to_kind(tag), expected, "mismatch for tag '{tag}'");
        }
    }

    /// Unknown variant tags should fall back to `"unknown"` for
    /// forward-compatibility with new `rustdoc-types` releases.
    #[test]
    fn tag_to_kind_unknown_falls_back() {
        assert_eq!(tag_to_kind("future_variant"), "unknown");
        assert_eq!(tag_to_kind(""), "unknown");
    }

    /// Deserialise a newtype/struct variant `{"function": {...}}`.
    #[test]
    fn deserialize_kind_tag_map_variant() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "deserialize_kind_tag")]
            inner: String,
        }
        let json = r#"{"inner": {"function": {"sig": {}, "generics": {}}}}"#;
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert_eq!(w.inner, "function");
    }

    /// Deserialise a unit variant `"extern_type"`.
    #[test]
    fn deserialize_kind_tag_string_variant() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "deserialize_kind_tag")]
            inner: String,
        }
        let json = r#"{"inner": "extern_type"}"#;
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert_eq!(w.inner, "extern_type");
    }

    /// Unknown variant tag deserialises as `"unknown"` without error.
    #[test]
    fn deserialize_kind_tag_unknown_variant() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(deserialize_with = "deserialize_kind_tag")]
            inner: String,
        }
        let json = r#"{"inner": {"future_variant": {"x": 1}}}"#;
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert_eq!(w.inner, "unknown");
    }
}
