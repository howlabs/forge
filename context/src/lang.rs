//! Language registry: maps file extensions to tree-sitter [`Language`]s.
//!
//! The registry is the only piece of the context engine that knows which
//! grammars are available.  It is consumed by [`crate::symbols::parse_symbols`]
//! to pick a grammar from a file path and is otherwise side-effect free.
//!
//! Adding a new language is a two-step process:
//!  1. add the grammar crate as a dependency in `Cargo.toml`,
//!  2. extend [`Lang::for_path`] with the matching extensions.
//!
//! All paths are matched on their extension (lowercased).  Files without a
//! matching extension return `None` from [`LanguageRegistry::language_for_path`].

use std::path::Path;

use tree_sitter::Language as TsLanguage;

/// Identifies a tree-sitter grammar that forge-context knows how to parse.
///
/// The variants map 1-to-1 to tree-sitter grammar crates.  The handle to
/// the underlying [`tree_sitter::Language`] is obtained on demand via
/// [`Lang::ts_language`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// Rust — see `tree-sitter-rust`.
    Rust,
    /// Python — see `tree-sitter-python`.
    Python,
    /// JavaScript — see `tree-sitter-javascript`.
    JavaScript,
    /// TypeScript — see `tree-sitter-typescript` (TS, not TSX).
    TypeScript,
    /// TSX (TypeScript with JSX) — see `tree-sitter-typescript`.
    Tsx,
    /// Go — see `tree-sitter-go`.
    Go,
    /// Java — see `tree-sitter-java`.
    Java,
    /// C++ — see `tree-sitter-cpp`.
    Cpp,
}

impl Lang {
    /// Resolve a [`Lang`] from a file path based on its extension.
    ///
    /// Returns `None` for paths without an extension or with an extension
    /// forge-context does not recognise.  Matching is case-insensitive.
    pub fn for_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "js" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "cpp" | "cxx" | "cc" | "h" | "hpp" | "c" => Some(Self::Cpp),
            _ => None,
        }
    }

    /// Get the underlying tree-sitter [`TsLanguage`] handle.
    pub fn ts_language(self) -> TsLanguage {
        match self {
            Self::Rust => tree_sitter_rust::language(),
            Self::Python => tree_sitter_python::language(),
            Self::JavaScript => tree_sitter_javascript::language(),
            Self::TypeScript => tree_sitter_typescript::language_typescript(),
            Self::Tsx => tree_sitter_typescript::language_tsx(),
            Self::Go => tree_sitter_go::language(),
            Self::Java => tree_sitter_java::language(),
            Self::Cpp => tree_sitter_cpp::language(),
        }
    }
}

/// Maps a file extension to a tree-sitter [`TsLanguage`].
///
/// The registry is a value type with no internal state.  It exists so
/// callers can be explicit about the registry they are using (helpful for
/// testing, dependency injection, and future "load grammars on demand"
/// work).
#[derive(Debug, Default, Clone, Copy)]
pub struct LanguageRegistry;

impl LanguageRegistry {
    /// Build a new registry.  Currently a no-op; provided for API symmetry
    /// and so future configuration can be threaded through here without
    /// breaking callers.
    pub fn new() -> Self {
        Self
    }

    /// Look up the tree-sitter [`TsLanguage`] for a given file path.
    ///
    /// Returns `None` for unsupported extensions — never an error.
    pub fn language_for_path(&self, path: &Path) -> Option<TsLanguage> {
        Lang::for_path(path).map(Lang::ts_language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rust_extension_resolves() {
        let p = PathBuf::from("foo/bar.rs");
        let l = LanguageRegistry::new().language_for_path(&p);
        assert!(l.is_some());
        assert_eq!(Lang::for_path(&p), Some(Lang::Rust));
    }

    #[test]
    fn python_extension_resolves() {
        let p = PathBuf::from("foo/bar.py");
        assert_eq!(Lang::for_path(&p), Some(Lang::Python));
    }

    #[test]
    fn typescript_and_tsx_are_distinct() {
        assert_eq!(
            Lang::for_path(&PathBuf::from("a.ts")),
            Some(Lang::TypeScript)
        );
        assert_eq!(Lang::for_path(&PathBuf::from("a.tsx")), Some(Lang::Tsx));
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert!(LanguageRegistry::new()
            .language_for_path(&PathBuf::from("a.txt"))
            .is_none());
        assert!(Lang::for_path(&PathBuf::from("README")).is_none());
    }

    #[test]
    fn matching_is_case_insensitive() {
        let p = PathBuf::from("FOO.RS");
        assert_eq!(Lang::for_path(&p), Some(Lang::Rust));
    }
    #[test]
    fn new_languages_resolve() {
        assert_eq!(Lang::for_path(&PathBuf::from("a.go")), Some(Lang::Go));
        assert_eq!(Lang::for_path(&PathBuf::from("a.java")), Some(Lang::Java));
        assert_eq!(Lang::for_path(&PathBuf::from("a.cpp")), Some(Lang::Cpp));
        assert_eq!(Lang::for_path(&PathBuf::from("a.h")), Some(Lang::Cpp));
    }
}
