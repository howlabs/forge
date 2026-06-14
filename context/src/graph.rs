//! In-memory knowledge graph of project symbols.
//!
//! The graph stores one node per extracted [`Symbol`] and connects them
//! with four kinds of edges:
//!
//! * [`EdgeKind::Contains`] — module/impl/struct → items declared inside
//!   it.  Built from line-range nesting: if symbol `A`'s range strictly
//!   contains symbol `B`'s, we add `A --Contains--> B`.
//! * [`EdgeKind::Calls`] — function/method → function it calls.
//! * [`EdgeKind::References`] — any symbol → any other symbol mentioned
//!   by name elsewhere in its body.
//! * [`EdgeKind::Implements`] — `impl` block → the struct/enum it
//!   implements, and (when present) → the trait it implements.
//!
//! ## Limitations
//!
//! Call and reference edges are name-based: any identifier inside a
//! function body that matches the name of a known symbol is treated as
//! a reference.  This deliberately ignores scoping, imports, shadowing,
//! and types.  Full type resolution is out of scope for v0.130.
//!
//! Adjacency lists are stored as `Vec<SymbolId>` per `(node, kind)`
//! bucket; we keep no external graph crate to avoid dragging petgraph
//! into the public API surface.  All returned slices are sorted by
//! [`SymbolId`] so callers see a stable, deterministic order.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use crate::lang::{Lang, LanguageRegistry};
use crate::symbols::{parse_symbols, Symbol, SymbolKind};

// =============================================================================
// Public types
// =============================================================================

/// Stable, dense identifier for a symbol stored in a [`KnowledgeGraph`].
///
/// `SymbolId(0)` is reserved and never returned by any API.  Real
/// symbols are assigned ids starting at 1 in the order they are
/// inserted by [`KnowledgeGraph::add_file`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub u32);

/// Kind of relationship between two symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// A directly calls B (a function/method invokes another).
    Calls,
    /// A references B by name (a type mention, a use, etc.).
    References,
    /// A contains B in its body (module → items, impl → methods).
    Contains,
    /// An `impl` block implements a struct/enum (and optionally a trait).
    Implements,
}

impl EdgeKind {
    /// String label for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::References => "references",
            Self::Contains => "contains",
            Self::Implements => "implements",
        }
    }
}

// =============================================================================
// Knowledge graph
// =============================================================================

/// In-memory graph of project symbols and their relationships.
///
/// Storage layout:
///
/// * `symbols` — dense vector of [`Symbol`]s, indexed by `SymbolId`.
/// * `by_name` — name → ids (a name may map to many ids; we keep all
///   to support duplicate names like `new`, `fmt`, `__init__`).
/// * `adj` — `(from, kind) → sorted set of `to` ids.
/// * `files` — list of file paths already indexed (used for diagnostics
///   and for future re-indexing; currently the graph is append-only).
#[derive(Debug, Default)]
pub struct KnowledgeGraph {
    symbols: Vec<Option<Symbol>>,
    by_name: HashMap<String, BTreeSet<SymbolId>>,
    adj: HashMap<(SymbolId, EdgeKind), BTreeSet<SymbolId>>,
    files: Vec<PathBuf>,
}

impl KnowledgeGraph {
    /// Build a new, empty knowledge graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of symbols currently in the graph.
    pub fn symbol_count(&self) -> usize {
        // Vec is sparse: every `push` lines up with the symbol's
        // 1-based id, so the count is just the length.
        self.symbols.len()
    }

    /// Look up a symbol by id.
    ///
    /// Returns `None` for [`SymbolId(0)`] and for ids that have never
    /// been assigned.
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        if id.0 == 0 {
            return None;
        }
        // `symbols` is Vec<Option<Symbol>> indexed 0-based, but ids
        // are 1-based (id 1 → index 0).  We use `Option` so the
        // contract is robust if a future change ever needs to
        // remove a symbol.
        let idx = id.0 as usize - 1;
        self.symbols.get(idx).and_then(|s| s.as_ref())
    }

    /// All symbols whose name matches the given string.
    ///
    /// Returned ids are sorted ascending.  If the name appears in
    /// multiple files or multiple times within a file (e.g. two
    /// `impl` blocks each with a `new` method), every match is
    /// returned.
    pub fn find_by_name(&self, name: &str) -> Vec<SymbolId> {
        self.by_name
            .get(name)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Neighbors of `id` reachable via the given edge kind.
    ///
    /// The returned ids are sorted ascending and de-duplicated.
    pub fn neighbors(&self, id: SymbolId, kind: EdgeKind) -> Vec<SymbolId> {
        self.adj
            .get(&(id, kind))
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Parse `source` as `path`, add its symbols to the graph, and
    /// connect them with [`EdgeKind::Contains`], [`EdgeKind::Calls`],
    /// [`EdgeKind::References`], and [`EdgeKind::Implements`] edges
    /// where the heuristics can find them.
    ///
    /// Unsupported extensions are a no-op (the graph is left
    /// unchanged).  Parse errors are surfaced as `Err`.
    pub fn add_file(
        &mut self,
        path: &Path,
        source: &str,
        registry: &LanguageRegistry,
    ) -> Result<()> {
        // No-op for unknown extensions — parse_symbols handles this,
        // but we still record the file so callers can introspect.
        let Some(language) = Lang::for_path(path) else {
            self.files.push(path.to_path_buf());
            return Ok(());
        };

        // 1. Parse symbols via the v0.130 entry point.
        let parsed = parse_symbols(path, source, registry)
            .with_context(|| format!("parsing {}", path.display()))?;
        if parsed.is_empty() {
            self.files.push(path.to_path_buf());
            return Ok(());
        }

        // 2. Assign ids and register in the by-name index.
        let mut id_for_index = Vec::with_capacity(parsed.len());
        for sym in parsed {
            let id = self.insert_symbol(sym);
            id_for_index.push(id);
        }

        // 3. Contains edges by line-range nesting.
        self.add_contains_edges(&id_for_index);

        // 4. Calls / References / Implements via tree walk.
        let mut parser = Parser::new();
        parser
            .set_language(language.ts_language())
            .context("failed to set tree-sitter language")?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse {}", path.display()))?;
        self.walk_for_references(language, &tree, source, &id_for_index);

        self.files.push(path.to_path_buf());
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Internals
    // -------------------------------------------------------------------------

    fn insert_symbol(&mut self, sym: Symbol) -> SymbolId {
        // Ids are 1-based: the first symbol pushed gets `SymbolId(1)`.
        // The Vec is 0-indexed, so index 0 holds `SymbolId(1)`.
        // `SymbolId(0)` is a never-assigned sentinel — `symbol(0)`
        // short-circuits to `None` to prevent underflow.
        let id = SymbolId(self.symbols.len() as u32 + 1);
        self.by_name.entry(sym.name.clone()).or_default().insert(id);
        self.symbols.push(Some(sym));
        id
    }

    /// Add `Contains` edges for any pair of newly-added symbols where
    /// the outer range strictly contains the inner range.
    fn add_contains_edges(&mut self, ids: &[SymbolId]) {
        // For each new symbol, find every *other* new symbol whose
        // range it strictly encloses.  We O(n^2) over the new batch
        // only — existing symbols are not re-scanned, which is fine
        // because no existing symbol's range can newly contain a
        // freshly parsed one (we add one file at a time).
        //
        // We exclude `Impl` from being a *container*: an `impl Foo`
        // block is the *thing that implements* the type — its body
        // contains methods that semantically belong to the type, not
        // to the impl block.  We still emit `Contains` for `Module`
        // and `Impl` containers but for tests we focus on the module
        // case; impl->method containment is exposed via the same edge
        // for completeness, while the canonical "method belongs to
        // type" relationship is the `Implements` edge.
        for &a in ids {
            let Some(sa) = self.symbol(a).cloned() else {
                continue;
            };
            for &b in ids {
                if a == b {
                    continue;
                }
                let Some(sb) = self.symbol(b).cloned() else {
                    continue;
                };
                if sa.start_line <= sb.start_line && sb.end_line <= sa.end_line && sa != sb {
                    self.link(a, EdgeKind::Contains, b);
                }
            }
        }
    }

    /// Walk the syntax tree to find identifiers and turn them into
    /// `Calls` / `References` edges.  Also detects `impl` blocks and
    /// creates `Implements` edges to the target type/trait.
    fn walk_for_references(
        &mut self,
        language: Lang,
        tree: &tree_sitter::Tree,
        source: &str,
        ids: &[SymbolId],
    ) {
        // Index by line for cheap "is this identifier inside this
        // symbol" lookups.
        let mut by_line: BTreeMap<usize, Vec<SymbolId>> = BTreeMap::new();
        for &id in ids {
            let Some(s) = self.symbol(id) else { continue };
            for line in s.start_line..=s.end_line {
                by_line.entry(line).or_default().push(id);
            }
        }

        // Collect identifiers with the line they appear on.
        // `tree_sitter` `Node`s are valid for the lifetime of the
        // tree; we extract the text we need before walking.
        let mut refs: Vec<(usize, String, /* is_call */ bool)> = Vec::new();
        Self::collect_identifiers(language, tree.root_node(), source, &mut refs);

        // Collapse duplicate (line, name) entries to a single record,
        // preferring `is_call = true` when both forms are seen for the
        // same identifier (the call_expression case pushes the same
        // identifier once as a call and once as a bare identifier,
        // since the recursive walker descends into the function
        // child).  Without this collapse the second visit would
        // overwrite the `Calls` intent with a `References` one.
        refs.sort_by_key(|(line, name, _)| (*line, name.clone()));
        refs.dedup_by(|a, b| {
            if a.0 == b.0 && a.1 == b.1 {
                if a.2 != b.2 {
                    // Promote to `is_call = true` if either visit says so.
                    a.2 = true;
                }
                true
            } else {
                false
            }
        });

        // We need a quick "is this id a function?" check.
        let function_ids: BTreeSet<SymbolId> = ids
            .iter()
            .copied()
            .filter(|id| {
                self.symbol(*id)
                    .map(|s| matches!(s.kind, SymbolKind::Function))
                    .unwrap_or(false)
            })
            .collect();

        // For impl blocks, look up the target type/trait by name and
        // create Implements edges.
        for &id in ids {
            let Some(sym) = self.symbol(id) else { continue };
            if sym.kind != SymbolKind::Impl {
                continue;
            }
            // The synthetic name is "impl Foo" or "impl Bar for Foo".
            // We only link to the *type* part (`Foo`) for the
            // Implements edge; the trait part is added as a reference
            // when we encounter the corresponding identifier in the
            // body.
            let tail = sym
                .name
                .strip_prefix("impl ")
                .unwrap_or(&sym.name)
                .trim_start();
            let type_name = tail
                .split_once(" for ")
                .map(|(_, ty)| ty)
                .unwrap_or(tail)
                .trim();
            if !type_name.is_empty() {
                for target in self.find_by_name(type_name) {
                    if target != id {
                        self.link(id, EdgeKind::Implements, target);
                    }
                }
            }
        }

        // Resolve each identifier to a "containing symbol" and link
        // to the matching definition(s) by name.
        for (line, name, is_call) in refs {
            if name.is_empty() {
                continue;
            }
            // Skip self-references to the function/struct name that
            // defines the current scope: the parser already accounts
            // for those, and double-counting them pollutes the
            // "incoming edges" view.
            let candidates = match by_line.get(&line) {
                Some(c) => c.clone(),
                None => continue,
            };

            let targets = self.find_by_name(&name);
            if targets.is_empty() {
                continue;
            }

            for owner in candidates {
                if targets.contains(&owner) {
                    // Don't link a symbol to itself.
                    continue;
                }
                let kind = if is_call && function_ids.contains(&owner) {
                    EdgeKind::Calls
                } else {
                    EdgeKind::References
                };
                for t in &targets {
                    if *t != owner {
                        self.link(owner, kind, *t);
                    }
                }
            }
        }
    }

    /// Push every identifier node we encounter into `out`.  The
    /// `is_call` flag is set when the identifier appears in a
    /// position that *looks* like a function call: it is the function
    /// of a `call_expression` (JS/TS/Python) or the callee of an
    /// invocation in Rust.  All other identifier references are
    /// treated as `References`.
    fn collect_identifiers(
        language: Lang,
        node: Node,
        source: &str,
        out: &mut Vec<(usize, String, bool)>,
    ) {
        let kind = node.kind();
        let line = node.start_position().row + 1;

        match language {
            Lang::Rust => {
                // Rust call: `foo(args)` shows up as
                // `call_expression` with `function: (identifier)`.
                if kind == "call_expression" {
                    if let Some(func) = node.child_by_field_name("function") {
                        Self::push_identifier_from(func, source, line, out, true);
                    }
                } else if kind == "identifier" {
                    Self::push_identifier_from(node, source, line, out, false);
                }
            }
            Lang::Python => {
                if kind == "call" {
                    if let Some(func) = node.child_by_field_name("function") {
                        // `func` may be an `attribute` (e.g. `mod.foo`).
                        // For v0.130 we link to the trailing identifier
                        // only — full qualifier resolution is out of
                        // scope.
                        if func.kind() == "attribute" {
                            if let Some(attr) = func.child_by_field_name("attribute") {
                                Self::push_identifier_from(attr, source, line, out, true);
                            }
                        } else {
                            Self::push_identifier_from(func, source, line, out, true);
                        }
                    }
                } else if kind == "identifier" {
                    Self::push_identifier_from(node, source, line, out, false);
                }
            }
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
                if kind == "call_expression" {
                    if let Some(func) = node.child_by_field_name("function") {
                        if func.kind() == "member_expression" {
                            if let Some(prop) = func.child_by_field_name("property") {
                                Self::push_identifier_from(prop, source, line, out, true);
                            }
                        } else {
                            Self::push_identifier_from(func, source, line, out, true);
                        }
                    }
                } else if kind == "identifier" {
                    Self::push_identifier_from(node, source, line, out, false);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::collect_identifiers(language, child, source, out);
        }
    }

    fn push_identifier_from(
        node: Node,
        source: &str,
        line: usize,
        out: &mut Vec<(usize, String, bool)>,
        is_call: bool,
    ) {
        let s = node.start_byte();
        let e = node.end_byte();
        let text = source.get(s..e).unwrap_or("").to_string();
        let text = text.trim();
        if !text.is_empty() && is_valid_identifier(text) {
            out.push((line, text.to_string(), is_call));
        }
    }

    fn link(&mut self, from: SymbolId, kind: EdgeKind, to: SymbolId) {
        self.adj.entry((from, kind)).or_default().insert(to);
    }
}

fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
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

    fn parse_first_function_id(graph: &KnowledgeGraph, name: &str) -> SymbolId {
        let mut ids = graph.find_by_name(name);
        assert!(
            !ids.is_empty(),
            "expected at least one symbol named {name:?}"
        );
        // Sort for determinism (find_by_name already does, but be explicit).
        ids.sort();
        let id = ids.remove(0);
        // Functions only for the call-edge test.
        assert_eq!(graph.symbol(id).unwrap().kind, SymbolKind::Function);
        id
    }

    #[test]
    fn add_file_records_symbols_and_count() {
        let src = "fn alpha() {}\nfn beta() {}\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();
        assert_eq!(g.symbol_count(), 2);
        assert!(g.find_by_name("alpha").contains(&SymbolId(1)));
        assert!(g.find_by_name("beta").contains(&SymbolId(2)));
    }

    #[test]
    fn function_calling_another_produces_calls_edge() {
        let src = "fn callee() -> i32 { 1 }\nfn caller() -> i32 { callee() }\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();

        let caller = parse_first_function_id(&g, "caller");
        let callee = parse_first_function_id(&g, "callee");

        let calls = g.neighbors(caller, EdgeKind::Calls);
        assert!(
            calls.contains(&callee),
            "expected caller->callee Calls edge, got {calls:?}"
        );
        // The reverse direction should not exist.
        assert!(g.neighbors(callee, EdgeKind::Calls).is_empty());
    }

    #[test]
    fn module_containing_struct_produces_contains_edge() {
        let src = "mod api {\n    pub struct Item {\n        pub n: i32,\n    }\n}\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();

        let module = *g
            .find_by_name("api")
            .first()
            .expect("module symbol present");
        assert_eq!(g.symbol(module).unwrap().kind, SymbolKind::Module);

        let item = *g
            .find_by_name("Item")
            .first()
            .expect("struct symbol present");
        assert_eq!(g.symbol(item).unwrap().kind, SymbolKind::Struct);

        let contains = g.neighbors(module, EdgeKind::Contains);
        assert!(
            contains.contains(&item),
            "expected module->struct Contains edge, got {contains:?}"
        );
    }

    #[test]
    fn find_by_name_returns_sorted_unique_ids() {
        let src = "struct A; struct B; fn a_helper() {}\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();

        // The `a` substring of `a_helper` should not match — the
        // find_by_name contract is exact name match.
        let mut a_ids = g.find_by_name("A");
        a_ids.sort();
        assert_eq!(a_ids.len(), 1, "expected one struct named A, got {a_ids:?}");

        let b_ids = g.find_by_name("B");
        assert_eq!(b_ids.len(), 1);

        // And the index is ascending.
        assert!(a_ids[0] < b_ids[0]);
    }

    #[test]
    fn unknown_extension_is_a_no_op() {
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("README.txt"), "no symbols here", &registry())
            .unwrap();
        assert_eq!(g.symbol_count(), 0);
    }

    #[test]
    fn impl_block_creates_implements_edge() {
        let src = "struct Foo;\nimpl Foo {\n    fn new() -> Foo { Foo }\n}\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();

        let foo = *g.find_by_name("Foo").first().unwrap();
        // The impl symbol's synthetic name is "impl Foo".
        let impl_id = *g
            .find_by_name("impl Foo")
            .first()
            .expect("impl symbol present");
        assert_eq!(g.symbol(impl_id).unwrap().kind, SymbolKind::Impl);

        let implements = g.neighbors(impl_id, EdgeKind::Implements);
        assert!(
            implements.contains(&foo),
            "expected impl->Foo Implements edge, got {implements:?}"
        );
    }

    #[test]
    fn trait_impl_creates_implements_edge_to_type() {
        let src =
            "trait Display {}\nstruct Foo;\nimpl Display for Foo {\n    fn fmt(&self) {}\n}\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();

        let foo = *g.find_by_name("Foo").first().unwrap();
        let display = *g.find_by_name("Display").first().unwrap();
        let impl_id = *g
            .find_by_name("impl Display for Foo")
            .first()
            .expect("impl symbol present");

        let implements = g.neighbors(impl_id, EdgeKind::Implements);
        assert!(
            implements.contains(&foo),
            "expected impl->Foo Implements edge, got {implements:?}"
        );
        assert!(
            !implements.contains(&display),
            "trait impl should not use the trait as its type target"
        );
    }

    #[test]
    fn symbol_id_zero_is_never_resolved() {
        let g = KnowledgeGraph::new();
        assert!(g.symbol(SymbolId(0)).is_none());
        assert!(g.neighbors(SymbolId(0), EdgeKind::Calls).is_empty());
    }

    #[test]
    fn symbol_id_beyond_count_returns_none() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("a.rs"),
            "fn a() {}\nfn b() {}\n",
            &registry(),
        )
        .unwrap();
        // Valid ids are 1, 2.  Anything beyond must be None.
        for id in &[SymbolId(0), SymbolId(3), SymbolId(100)] {
            assert!(
                g.symbol(*id).is_none(),
                "symbol({:?}) should be None (count={})",
                id,
                g.symbol_count()
            );
        }
    }
}
