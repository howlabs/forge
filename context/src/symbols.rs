//! Tree-sitter based symbol extraction.
//!
//! Given a source file and a [`LanguageRegistry`], [`parse_symbols`]
//! walks the syntax tree produced by the appropriate grammar and emits a
//! flat list of [`Symbol`]s.
//!
//! The set of kinds we surface is intentionally small — the six variants
//! of [`SymbolKind`] — and was chosen to match the public API
//! contract for the v0.130 semantic context engine.  Languages that lack
//! some of these constructs (e.g. JavaScript has no `struct`) simply
//! yield fewer symbols; unsupported extensions yield none.
//!
//! The function is fully synchronous, allocation-light, and deterministic:
//! the same input always produces the same output in the same order.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::lang::{Lang, LanguageRegistry};

// =============================================================================
// Public types
// =============================================================================

/// Kind of a symbol extracted from source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SymbolKind {
    /// Free function / method.
    Function,
    /// Struct / class declaration.
    Struct,
    /// Enum declaration.
    Enum,
    /// Trait / interface declaration.
    Trait,
    /// `impl` block (Rust only at the moment).
    Impl,
    /// Module / namespace / top-level file scope.
    Module,
    /// Interface declaration (Java, TypeScript, Go).
    Interface,
    /// Namespace declaration (C++).
    Namespace,
    /// Constant declaration (Go, C++).
    Const,
}

impl SymbolKind {
    /// String label used for diagnostics and (later) serialisation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Module => "module",
            Self::Interface => "interface",
            Self::Namespace => "namespace",
            Self::Const => "const",
        }
    }
}

/// A single symbol extracted from a source file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    /// Symbol name, e.g. `"foo"` or `"ContextIndex::upsert_file"`.
    pub name: String,
    /// What kind of declaration this is.
    pub kind: SymbolKind,
    /// File the symbol was extracted from.
    pub file: std::path::PathBuf,
    /// First source line (1-indexed) containing the symbol.
    pub start_line: usize,
    /// Last source line (1-indexed) containing the symbol.
    pub end_line: usize,
    /// One-line signature / declaration text, trimmed.
    pub signature: String,
}

// =============================================================================
// Public entry point
// =============================================================================

/// Parse a source file and return all top-level symbols it contains.
///
/// * Unrecognised extensions return `Ok(Vec::new())` — never an error.
/// * Parse errors and unparseable fragments are surfaced as `Err`.
/// * The function does no I/O; pass the file contents in `source`.
pub fn parse_symbols(
    path: &Path,
    source: &str,
    registry: &LanguageRegistry,
) -> Result<Vec<Symbol>> {
    // The registry is part of the public API for symmetry and future
    // configuration (e.g. loading grammars on demand, custom mappings).
    // We still resolve the language from the path so the contract is
    // observable: passing a registry with custom rules must not change
    // the extension-based dispatch.
    let _ = registry;
    let Some(language) = Lang::for_path(path) else {
        return Ok(Vec::new());
    };

    let mut parser = Parser::new();
    parser
        .set_language(language.ts_language())
        .context("failed to set tree-sitter language")?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("tree-sitter failed to parse {}", path.display()))?;

    let query_src = query_for(language);
    let query = Query::new(language.ts_language(), query_src)
        .with_context(|| format!("invalid query for {:?}", language))?;

    let mut cursor = QueryCursor::new();
    let mut out: Vec<Symbol> = Vec::new();

    let symbol_idx = query
        .capture_index_for_name("symbol")
        .ok_or_else(|| anyhow!("query is missing @symbol capture"))?;

    for m in cursor.matches(&query, tree.root_node(), source.as_bytes()) {
        // Each match corresponds to exactly one AST node — there is
        // no need to deduplicate.  Real-world code has many symbols
        // with the same name (`new`, `fmt`, `default`, `__init__`,
        // etc.) spread across different impls, modules, or classes;
        // dropping them by `(kind, name)` would silently destroy
        // legitimate entries and corrupt the downstream index.
        for capture in m.captures {
            if capture.index == symbol_idx {
                // Error resilience: skip ERROR nodes generated during partial parses
                if capture.node.is_error() {
                    break;
                }
                
                if let Some(sym) = build_symbol(path, source, language, capture.node) {
                    out.push(sym);
                }
                break;
            }
        }
    }

    Ok(out)
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Pick the right embedded query for a language.
fn query_for(language: Lang) -> &'static str {
    match language {
        Lang::Rust => include_str!("../queries/rust.scm"),
        Lang::Python => include_str!("../queries/python.scm"),
        Lang::JavaScript => include_str!("../queries/javascript.scm"),
        Lang::TypeScript | Lang::Tsx => include_str!("../queries/typescript.scm"),
        Lang::Go => include_str!("../queries/go.scm"),
        Lang::Java => include_str!("../queries/java.scm"),
        Lang::Cpp => include_str!("../queries/cpp.scm"),
    }
}

/// Decide what kind a node represents for a given language, and pull
/// the name out of it.
fn classify(language: Lang, node: Node, source: &str) -> Option<(SymbolKind, String)> {
    let kind = node.kind();
    match language {
        Lang::Rust => match kind {
            "function_item" => Some((SymbolKind::Function, child_text(node, "name", source))),
            "struct_item" => Some((SymbolKind::Struct, child_text(node, "name", source))),
            "enum_item" => Some((SymbolKind::Enum, child_text(node, "name", source))),
            "trait_item" => Some((SymbolKind::Trait, child_text(node, "name", source))),
            "impl_item" => {
                // Try to compose a "impl Trait for Type" style name.
                let trait_ = node.child_by_field_name("trait");
                let ty = node.child_by_field_name("type");
                let name = match (trait_, ty) {
                    (Some(t), Some(y)) => {
                        format!("impl {} for {}", node_text(t, source), node_text(y, source))
                    }
                    (None, Some(y)) => format!("impl {}", node_text(y, source)),
                    (Some(t), None) => format!("impl {}", node_text(t, source)),
                    (None, None) => "impl".to_string(),
                };
                Some((SymbolKind::Impl, name))
            }
            "mod_item" => Some((SymbolKind::Module, child_text(node, "name", source))),
            _ => None,
        },
        Lang::Python => match kind {
            "function_definition" => Some((SymbolKind::Function, child_text(node, "name", source))),
            // Map Python classes to Struct — the public API has no
            // separate Class kind.
            "class_definition" => Some((SymbolKind::Struct, child_text(node, "name", source))),
            "decorated_definition" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if let Some(symbol) = match child.kind() {
                        "function_definition" | "class_definition" => {
                            classify(language, child, source)
                        }
                        _ => None,
                    } {
                        return Some(symbol);
                    }
                }
                None
            }
            "module" => Some((SymbolKind::Module, "<module>".to_string())),
            _ => None,
        },
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => match kind {
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                Some((SymbolKind::Function, child_text(node, "name", source)))
            }
            "method_definition" => Some((SymbolKind::Function, child_text(node, "name", source))),
            "class_declaration" | "abstract_class_declaration" => {
                Some((SymbolKind::Struct, child_text(node, "name", source)))
            }
            "enum_declaration" => Some((SymbolKind::Enum, child_text(node, "name", source))),
            "interface_declaration" => Some((SymbolKind::Interface, child_text(node, "name", source))),
            "module" => Some((SymbolKind::Module, child_text(node, "name", source))),
            "namespace_declaration" => {
                Some((SymbolKind::Namespace, child_text(node, "name", source)))
            }
            "program" => Some((SymbolKind::Module, "<program>".to_string())),
            _ => None,
        },
        Lang::Go => match kind {
            "function_declaration" | "method_declaration" => {
                Some((SymbolKind::Function, child_text(node, "name", source)))
            }
            "type_spec" => {
                let type_kind = node.child_by_field_name("type")?.kind();
                let name = child_text(node, "name", source);
                match type_kind {
                    "struct_type" => Some((SymbolKind::Struct, name)),
                    "interface_type" => Some((SymbolKind::Interface, name)),
                    _ => None,
                }
            }
            "const_spec" => {
                let name = child_text(node, "name", source);
                if !name.is_empty() {
                    Some((SymbolKind::Const, name))
                } else {
                    None
                }
            }
            _ => None,
        },
        Lang::Java => match kind {
            "class_declaration" => Some((SymbolKind::Struct, child_text(node, "name", source))),
            "interface_declaration" => Some((SymbolKind::Interface, child_text(node, "name", source))),
            "enum_declaration" => Some((SymbolKind::Enum, child_text(node, "name", source))),
            "method_declaration" | "constructor_declaration" => {
                Some((SymbolKind::Function, child_text(node, "name", source)))
            }
            _ => None,
        },
        Lang::Cpp => match kind {
            "class_specifier" | "struct_specifier" => {
                Some((SymbolKind::Struct, child_text(node, "name", source)))
            }
            "enum_specifier" => Some((SymbolKind::Enum, child_text(node, "name", source))),
            "function_definition" => {
                let decl = node.child_by_field_name("declarator");
                let name = if let Some(decl) = decl {
                    // Try to dig out the identifier
                    let mut name = String::new();
                    // simple heuristic: find the innermost identifier or scoped_identifier
                    let mut current = decl;
                    while let Some(child) = current.child_by_field_name("declarator") {
                        current = child;
                    }
                    if let Some(id) = current.child(0) {
                        if id.kind() == "identifier" || id.kind() == "scoped_identifier" {
                            name = node_text(id, source);
                        }
                    }
                    if name.is_empty() {
                        node_text(current, source)
                    } else {
                        name
                    }
                } else {
                    "".to_string()
                };
                Some((SymbolKind::Function, name))
            }
            "namespace_definition" => {
                Some((SymbolKind::Namespace, child_text(node, "name", source)))
            }
            _ => None,
        },
    }
}

fn child_text(node: Node, field: &str, source: &str) -> String {
    node.child_by_field_name(field)
        .map(|n| node_text(n, source))
        .unwrap_or_default()
}

/// Extract the source text for a node.
pub fn node_text(node: Node, source: &str) -> String {
    let s = node.start_byte();
    let e = node.end_byte();
    source.get(s..e).unwrap_or("").to_string()
}

fn first_line(text: &str) -> String {
    // Take the first line, strip trailing whitespace, then collapse
    // internal runs of whitespace so the signature is one tidy line.
    let line = text.lines().next().unwrap_or("").trim_end();
    let mut out = String::with_capacity(line.len());
    let mut prev_space = false;
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

fn build_symbol(path: &Path, source: &str, language: Lang, node: Node) -> Option<Symbol> {
    let (kind, name) = classify(language, node, source)?;
    let raw = node_text(node, source);
    let signature = first_line(&raw);
    let start = node.start_position().row + 1;
    // tree-sitter ranges use a *half-open* convention for byte
    // offsets (so `end_byte` is one past the last byte) but a
    // *closed* convention for row/column.  In practice `end_position`
    // reports the row of the last byte, not one past it.  For
    // declarations that end on a newline, the last byte is `\n`, so
    // the reported row is the previous line — i.e. the line of the
    // `}`.  For a single-line declaration with no trailing newline
    // the last byte is `}` (or whatever the final token is), and
    // `end_position().row` is still that line.
    //
    // 1-indexed: add 1.  Clamp to `start` so we never return an
    // empty range if the start and end land on the same row
    // (defensive — should not happen for the nodes we capture).
    let end = node.end_position().row + 1;
    Some(Symbol {
        name,
        kind,
        file: path.to_path_buf(),
        start_line: start,
        end_line: end.max(start),
        signature,
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn registry() -> LanguageRegistry {
        LanguageRegistry::new()
    }

    #[test]
    fn rust_struct_and_function_are_extracted() {
        let src = "struct Point {\n    x: i32,\n    y: i32,\n}\n\nfn origin() -> Point {\n    Point { x: 0, y: 0 }\n}\n";
        let path = PathBuf::from("lib.rs");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");

        assert_eq!(syms.len(), 2, "expected 2 symbols, got {syms:#?}");

        let s0 = &syms[0];
        assert_eq!(s0.name, "Point");
        assert_eq!(s0.kind, SymbolKind::Struct);
        assert_eq!(s0.start_line, 1);
        assert_eq!(s0.end_line, 4);

        let s1 = &syms[1];
        assert_eq!(s1.name, "origin");
        assert_eq!(s1.kind, SymbolKind::Function);
        assert_eq!(s1.start_line, 6);
        assert!(
            s1.end_line >= 8,
            "end_line should cover the body: {}",
            s1.end_line
        );
        assert!(s1.signature.starts_with("fn origin"));
    }

    #[test]
    fn python_class_and_def_are_extracted() {
        let src = "class Greeter:\n    def __init__(self, name):\n        self.name = name\n\n    def greet(self):\n        return f'hello {self.name}'\n\ndef make_greeter(name):\n    return Greeter(name)\n";
        let path = PathBuf::from("app.py");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");

        assert!(syms.len() >= 3, "expected >=3 symbols, got {syms:#?}");

        // Don't rely on output order: just assert presence of the
        // three shapes the public API guarantees — class as Struct,
        // and at least one free function (the module-level def).
        assert!(syms
            .iter()
            .any(|s| s.name == "Greeter" && s.kind == SymbolKind::Struct));
        assert!(syms
            .iter()
            .any(|s| s.name == "make_greeter" && s.kind == SymbolKind::Function));
        // Greeter starts on line 1.
        assert!(syms
            .iter()
            .any(|s| s.name == "Greeter" && s.start_line == 1));
    }

    #[test]
    fn decorated_python_functions_are_functions() {
        let src = "@pytest.fixture\ndef client():\n    return object()\n\n@dataclass\nclass Config:\n    value: str\n";
        let path = PathBuf::from("test_app.py");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");

        assert!(syms
            .iter()
            .any(|s| s.name == "client" && s.kind == SymbolKind::Function));
        assert!(syms
            .iter()
            .any(|s| s.name == "Config" && s.kind == SymbolKind::Struct));
        assert!(
            syms.iter().all(|s| !s.name.is_empty()),
            "decorated definitions must not emit empty symbols: {syms:#?}"
        );
    }

    #[test]
    fn unsupported_extension_returns_empty_vec() {
        let path = PathBuf::from("notes.txt");
        let syms = parse_symbols(&path, "hello world", &registry()).expect("no error");
        assert!(
            syms.is_empty(),
            "expected no symbols for .txt, got {syms:#?}"
        );
    }

    #[test]
    fn rust_enum_trait_and_module_kind_round_trip() {
        let src = "mod api {\n    pub trait Speak {\n        fn speak(&self) -> String;\n    }\n    pub enum Pet { Dog, Cat }\n}\n";
        let path = PathBuf::from("lib.rs");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");
        let kinds: Vec<_> = syms.iter().map(|s| s.kind).collect();
        assert!(
            kinds.contains(&SymbolKind::Module),
            "missing module: {kinds:?}"
        );
        assert!(
            kinds.contains(&SymbolKind::Trait),
            "missing trait: {kinds:?}"
        );
        assert!(kinds.contains(&SymbolKind::Enum), "missing enum: {kinds:?}");
    }

    /// Regression: two `impl` blocks each defining a method with the
    /// same name (`new`) must both survive extraction.  An earlier
    /// draft of this module deduped symbols by `(kind, name)` and
    /// would silently drop all but the first occurrence — corrupting
    /// the index for any code with `new`, `fmt`, `default`, etc.
    #[test]
    fn same_named_methods_in_different_impls_are_all_kept() {
        let src =
            "struct A; struct B;\nimpl A { fn new() -> A { A } }\nimpl B { fn new() -> B { B } }\n";
        let path = PathBuf::from("x.rs");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");
        let news: Vec<_> = syms.iter().filter(|s| s.name == "new").collect();
        assert_eq!(news.len(), 2, "both new() must survive, got {syms:#?}");
        // And both should be functions, with different bodies.
        assert!(news.iter().all(|s| s.kind == SymbolKind::Function));
        assert_ne!(news[0].start_line, news[1].start_line);
    }
    #[test]
    fn go_symbols_are_extracted() {
        let src = "package main\n\ntype Handler interface {\n\tServeHTTP()\n}\n\ntype Server struct {\n\tport int\n}\n\nfunc (s *Server) ServeHTTP() {}\n\nconst Version = \"1.0.0\"\n";
        let path = PathBuf::from("main.go");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");
        
        assert!(syms.iter().any(|s| s.name == "Handler" && s.kind == SymbolKind::Interface));
        assert!(syms.iter().any(|s| s.name == "Server" && s.kind == SymbolKind::Struct));
        assert!(syms.iter().any(|s| s.name == "ServeHTTP" && s.kind == SymbolKind::Function));
        assert!(syms.iter().any(|s| s.name == "Version" && s.kind == SymbolKind::Const));
    }

    #[test]
    fn java_symbols_are_extracted() {
        let src = "package com.example;\n\npublic interface Runnable { void run(); }\n\npublic class App implements Runnable {\n    public enum Status { OK, ERROR }\n    public App() {}\n    public void run() {}\n}\n";
        let path = PathBuf::from("App.java");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");
        
        assert!(syms.iter().any(|s| s.name == "Runnable" && s.kind == SymbolKind::Interface));
        assert!(syms.iter().any(|s| s.name == "App" && s.kind == SymbolKind::Struct));
        assert!(syms.iter().any(|s| s.name == "Status" && s.kind == SymbolKind::Enum));
        assert!(syms.iter().any(|s| s.name == "App" && s.kind == SymbolKind::Function)); // Constructor
        assert!(syms.iter().any(|s| s.name == "run" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn cpp_symbols_are_extracted() {
        let src = "namespace math {\n    class Vector {};\n    struct Point { int x, y; };\n    enum Color { RED, BLUE };\n    void draw(Point p) {}\n}\n";
        let path = PathBuf::from("math.cpp");
        let syms = parse_symbols(&path, src, &registry()).expect("parses");
        
        assert!(syms.iter().any(|s| s.name == "math" && s.kind == SymbolKind::Namespace));
        assert!(syms.iter().any(|s| s.name == "Vector" && s.kind == SymbolKind::Struct));
        assert!(syms.iter().any(|s| s.name == "Point" && s.kind == SymbolKind::Struct));
        assert!(syms.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum));
        assert!(syms.iter().any(|s| s.name == "draw" && s.kind == SymbolKind::Function));
    }
}
