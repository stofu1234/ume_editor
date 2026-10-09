//! UI-independent core of ume_editor.
//!
//! The buffer, encodings, search, and editing commands live here. This crate
//! must not depend on any UI crate (see `CLAUDE.md`).

/// Returns the crate version, so that the workspace has something to build
/// and test until the real modules land in Phase 1.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_manifest() {
        assert_eq!(version(), "0.0.0");
    }
}
