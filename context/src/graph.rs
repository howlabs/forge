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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct SymbolId(pub u32);

/// Kind of relationship between two symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeGraph {
    symbols: Vec<Option<Symbol>>,
    by_name: HashMap<String, BTreeSet<SymbolId>>,
    #[serde(with = "adj_serde")]
    adj: HashMap<(SymbolId, EdgeKind), BTreeSet<SymbolId>>,
    files: Vec<PathBuf>,
    // ponytail: qualified name → symbol id. Built additively in add_file().
    #[serde(default)]
    qualified_names: HashMap<String, SymbolId>,
}

mod adj_serde {
    use super::{SymbolId, EdgeKind};
    use std::collections::{HashMap, BTreeSet};
    use serde::{Serialize, Serializer, Deserialize, Deserializer};

    pub fn serialize<S>(adj: &HashMap<(SymbolId, EdgeKind), BTreeSet<SymbolId>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let vec: Vec<_> = adj.iter().collect();
        vec.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<(SymbolId, EdgeKind), BTreeSet<SymbolId>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let vec: Vec<((SymbolId, EdgeKind), BTreeSet<SymbolId>)> = Vec::deserialize(deserializer)?;
        Ok(vec.into_iter().collect())
    }
}

impl KnowledgeGraph {
    /// Build a new, empty knowledge graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the graph from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let graph: Self = serde_json::from_str(&data)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(graph)
    }

    /// Save the graph to disk atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_string(self)
            .with_context(|| format!("serializing graph"))?;
        let temp_path = path.with_extension("tmp");
        std::fs::write(&temp_path, data)
            .with_context(|| format!("writing temp file {}", temp_path.display()))?;
        std::fs::rename(&temp_path, path)
            .with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }

    /// Number of live (non-removed) symbols in the graph.
    pub fn symbol_count(&self) -> usize {
        self.symbols.iter().filter(|s| s.is_some()).count()
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

        // 3.5. Compute qualified names for new symbols.
        let imports = Self::parse_imports(path, source);
        for &id in &id_for_index {
            let qname = self.compute_qname(id);
            self.qualified_names.insert(qname, id);
        }

        // 4. Calls / References / Implements via tree walk.
        let mut parser = Parser::new();
        parser
            .set_language(language.ts_language())
            .context("failed to set tree-sitter language")?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse {}", path.display()))?;
        self.walk_for_references(language, &tree, source, &id_for_index, &imports);

        self.files.push(path.to_path_buf());
        Ok(())
    }

    /// Remove all symbols and edges associated with a specific file.
    pub fn remove_file(&mut self, path: &Path) {
        let mut ids_to_remove = Vec::new();
        for (i, sym_opt) in self.symbols.iter().enumerate() {
            if let Some(sym) = sym_opt {
                if sym.file == path {
                    ids_to_remove.push(SymbolId(i as u32 + 1));
                }
            }
        }

        for id in &ids_to_remove {
            let sym_name = self.symbol(*id).map(|s| s.name.clone());
            if let Some(name) = sym_name {
                if let Some(set) = self.by_name.get_mut(&name) {
                    set.remove(id);
                    if set.is_empty() {
                        self.by_name.remove(&name);
                    }
                }
            }
            self.adj.retain(|(from, _), _| from != id);
            for set in self.adj.values_mut() {
                set.remove(id);
            }
            self.symbols[id.0 as usize - 1] = None;
        }

        // ponytail: clean qualified names for removed symbols
        ids_to_remove.iter().for_each(|id| {
            self.qualified_names.retain(|_, &mut sid| sid != *id);
        });

        self.files.retain(|p| p != path);
    }

    // =========================================================================
    // Qualified names + import resolution (§4)
    // =========================================================================

    /// Find the immediate parent of `id` via the tightest Contains edge.
    fn find_parent(&self, id: SymbolId) -> Option<SymbolId> {
        let _sym = self.symbol(id)?;
        let mut best: Option<(SymbolId, usize)> = None;

        for (from, kind) in self.adj.keys() {
            if *kind != EdgeKind::Contains {
                continue;
            }
            if let Some(children) = self.adj.get(&(*from, EdgeKind::Contains)) {
                if !children.contains(&id) {
                    continue;
                }
                if let Some(parent_sym) = self.symbol(*from) {
                    let span = parent_sym.end_line - parent_sym.start_line;
                    match best {
                        None => best = Some((*from, span)),
                        Some((_, best_span)) if span < best_span => {
                            best = Some((*from, span))
                        }
                        _ => {}
                    }
                }
            }
        }
        best.map(|(id, _)| id)
    }

    /// Compute the qualified name for a symbol by walking up the
    /// containment chain.  Format: `file_stem::parent::...::name`.
    fn compute_qname(&self, id: SymbolId) -> String {
        let mut parts = Vec::new();
        let mut current = Some(id);
        let mut visited = std::collections::HashSet::new();
        while let Some(cid) = current {
            if !visited.insert(cid) {
                break; // cycle guard
            }
            if let Some(s) = self.symbol(cid) {
                parts.push(s.name.clone());
            }
            current = self.find_parent(cid);
        }
        parts.reverse();

        if let Some(sym) = self.symbol(id) {
            let stem = sym
                .file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            parts.insert(0, stem.to_string());
        }
        parts.join("::")
    }

    /// Resolve a symbol by qualified name.
    pub fn resolve_qualified(&self, qname: &str) -> Option<SymbolId> {
        self.qualified_names.get(qname).copied()
    }

    /// Look up the qualified name for a given symbol id.
    pub fn resolve_qualified_by_id(&self, id: SymbolId) -> Option<String> {
        self.qualified_names
            .iter()
            .find(|(_, &sid)| sid == id)
            .map(|(qname, _)| qname.clone())
    }

    /// All symbols that call the target identified by `qname`.
    pub fn callers_of(&self, qname: &str) -> Vec<SymbolId> {
        let Some(target) = self.qualified_names.get(qname) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for ((from, kind), targets) in &self.adj {
            if *kind == EdgeKind::Calls && targets.contains(target) {
                out.push(*from);
            }
        }
        out
    }

    /// All symbols called by the symbol identified by `qname`.
    pub fn callees_of(&self, qname: &str) -> Vec<SymbolId> {
        let Some(&id) = self.qualified_names.get(qname) else {
            return Vec::new();
        };
        self.neighbors(id, EdgeKind::Calls)
    }

    /// All symbols that reference the target identified by `qname`
    /// (including Calls, which are a superset of References).
    pub fn references_to(&self, qname: &str) -> Vec<SymbolId> {
        let Some(target) = self.qualified_names.get(qname) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for ((from, kind), targets) in &self.adj {
            if (*kind == EdgeKind::References || *kind == EdgeKind::Calls)
                && targets.contains(target)
            {
                out.push(*from);
            }
        }
        out
    }

    // =========================================================================
    // Import parsing (best-effort, §4)
    // =========================================================================

    /// Best-effort extraction of `use`/`import`/`from...import` statements.
    /// Returns a map `local_name → imported_qualified_path`.
    fn parse_imports(path: &Path, source: &str) -> HashMap<String, String> {
        let mut imports = HashMap::new();
        let lang = match Lang::for_path(path) {
            Some(l) => l,
            None => return imports,
        };
        match lang {
            Lang::Rust => {
                for line in source.lines() {
                    let line = line.trim();
                    if !line.starts_with("use ") || !line.ends_with(';') {
                        continue;
                    }
                    let p = &line[4..line.len() - 1];
                    let segments: Vec<&str> = p.split("::").collect();
                    if let Some(last) = segments.last() {
                        if matches!(*last, "self" | "super" | "crate") {
                            continue;
                        }
                        imports.insert(last.to_string(), p.to_string());
                    }
                }
            }
            Lang::Python => {
                for line in source.lines() {
                    let line = line.trim();
                    if line.starts_with("import ") {
                        let module = &line[7..];
                        if let Some(name) = module.split('.').next() {
                            let name = name.split_whitespace().next().unwrap_or(name);
                            imports.insert(name.to_string(), module.to_string());
                        }
                    } else if let Some(rest) = line.strip_prefix("from ") {
                        if let Some(pos) = rest.find(" import ") {
                            let module = rest[..pos].trim();
                            let names_part = &rest[pos + 8..];
                            for name in names_part.split(',') {
                                let name = name
                                    .trim()
                                    .split(" as ")
                                    .next()
                                    .unwrap_or("")
                                    .trim();
                                if !name.is_empty() && name != "*" {
                                    imports.insert(name.to_string(), module.to_string());
                                }
                            }
                        }
                    }
                }
            }
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
                for line in source.lines() {
                    let line = line.trim();
                    if !line.starts_with("import ") || !line.contains(" from ") {
                        continue;
                    }
                    if let Some(from_pos) = line.find(" from ") {
                        let module = line[from_pos + 6..]
                            .trim()
                            .trim_matches(|c| c == '\'' || c == '"' || c == '`')
                            .trim_end_matches(';')
                            .trim();
                        let names_part = &line[7..from_pos];
                        if names_part.starts_with('{') {
                            for name in
                                names_part.trim_matches(|c| c == '{' || c == '}').split(',')
                            {
                                let name = name
                                    .trim()
                                    .split(" as ")
                                    .next()
                                    .unwrap_or("")
                                    .trim();
                                if !name.is_empty() {
                                    imports.insert(name.to_string(), module.to_string());
                                }
                            }
                        } else if !names_part.is_empty() {
                            let name = names_part
                                .trim()
                                .split(" as ")
                                .next()
                                .unwrap_or("")
                                .trim();
                            if !name.is_empty() {
                                imports.insert(name.to_string(), module.to_string());
                            }
                        }
                    }
                }
            }
            Lang::Go => {
                for line in source.lines() {
                    let line = line.trim();
                    if !line.starts_with("import ") {
                        continue;
                    }
                    let rest = &line[7..];
                    if rest.starts_with('"') {
                        let module =
                            rest.trim_matches(|c| c == '"' || c == '\'');
                        let name =
                            module.rsplit('/').next().unwrap_or(module);
                        imports.insert(name.to_string(), module.to_string());
                    } else if let Some(space_pos) = rest.find(' ') {
                        let alias = &rest[..space_pos];
                        let module = rest[space_pos + 1..]
                            .trim()
                            .trim_matches(|c| c == '"' || c == '\'');
                        imports.insert(alias.to_string(), module.to_string());
                    }
                }
            }
            Lang::Java => {
                for line in source.lines() {
                    let line = line.trim();
                    if !line.starts_with("import ") || !line.ends_with(';') {
                        continue;
                    }
                    let p = &line[7..line.len() - 1];
                    if let Some(name) = p.rsplit('.').next() {
                        imports.insert(name.to_string(), p.to_string());
                    }
                }
            }
            Lang::Cpp => {
                // C++ includes/using-declarations are not reliable import
                // indicators — skip for now.
            }
        }
        imports
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
    ///
    /// When an identifier matches an import alias, resolution is tried
    /// against the qualified-name index first (import-aware resolution).
    fn walk_for_references(
        &mut self,
        language: Lang,
        tree: &tree_sitter::Tree,
        source: &str,
        ids: &[SymbolId],
        imports: &HashMap<String, String>,
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

        // Collect local variable names to filter them out of references.
        let mut locals = std::collections::HashSet::new();
        Self::collect_locals(language, tree.root_node(), source, &mut locals);

        // Collect identifiers with the line they appear on.
        // `tree_sitter` `Node`s are valid for the lifetime of the
        // tree; we extract the text we need before walking.
        let mut refs: Vec<(usize, String, /* is_call */ bool)> = Vec::new();
        Self::collect_identifiers(language, tree.root_node(), source, &mut refs);

        // Filter out locals to prevent hallucinated edges to global functions with the same name.
        refs.retain(|(_, name, _)| !locals.contains(name));

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
        //
        // ponytail: import resolution is best-effort.  If the
        // identifier matches an import alias, the qualified-name
        // index is tried first.  On miss we fall back to plain
        // name-based lookup so callers in the same file still work.
        for (line, name, is_call) in refs {
            if name.is_empty() {
                continue;
            }
            let candidates = match by_line.get(&line) {
                Some(c) => c.clone(),
                None => continue,
            };

            // --- import-aware fast path ---
            if let Some(import_path) = imports.get(&name) {
                if let Some(&target_id) = self.qualified_names.get(import_path) {
                    for owner in &candidates {
                        if *owner == target_id {
                            continue;
                        }
                        let kind = if is_call && function_ids.contains(owner) {
                            EdgeKind::Calls
                        } else {
                            EdgeKind::References
                        };
                        self.link(*owner, kind, target_id);
                    }
                    continue;
                }
            }

            // --- fall back to name-based lookup ---
            let mut targets = self.find_by_name(&name);
            if targets.is_empty() {
                continue;
            }

            // Same-file scoping: if any target is in the same file as
            // the owner, prefer it (shadowing globals of the same name).
            for owner in &candidates {
                if let Some(owner_sym) = self.symbol(*owner) {
                    let owner_file = owner_sym.file.clone();
                    let same_file_targets: Vec<SymbolId> = targets
                        .iter()
                        .copied()
                        .filter(|&t| self.symbol(t).map_or(false, |s| s.file == owner_file))
                        .collect();
                    if !same_file_targets.is_empty() {
                        targets = same_file_targets;
                        break;
                    }
                }
            }

            for owner in candidates {
                if targets.contains(&owner) {
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
            Lang::Go | Lang::Java | Lang::Cpp => {
                if kind == "call_expression" || kind == "method_invocation" {
                    if let Some(func) = node.child_by_field_name("function").or_else(|| node.child_by_field_name("name")) {
                        // ponytail: handle selector/field exprs the same way JS handles member_expression
                        if func.kind() == "selector_expression" || func.kind() == "field_expression" {
                            if let Some(field) = func.child_by_field_name("field") {
                                Self::push_identifier_from(field, source, line, out, true);
                            }
                        } else {
                            Self::push_identifier_from(func, source, line, out, true);
                        }
                    }
                } else if kind == "selector_expression" || kind == "field_expression" {
                    if let Some(field) = node.child_by_field_name("field") {
                        Self::push_identifier_from(field, source, line, out, false);
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

    fn collect_locals(
        language: Lang,
        node: Node,
        source: &str,
        out: &mut std::collections::HashSet<String>,
    ) {
        let kind = node.kind();
        match language {
            Lang::Rust => {
                if kind == "let_declaration" {
                    if let Some(pat) = node.child_by_field_name("pattern") {
                        Self::extract_identifiers_from_pattern(pat, source, out);
                    }
                }
            }
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
                if kind == "lexical_declaration" || kind == "variable_declaration" {
                    Self::extract_identifiers_from_pattern(node, source, out);
                }
            }
            Lang::Python => {
                if kind == "assignment" {
                    if let Some(left) = node.child_by_field_name("left") {
                        Self::extract_identifiers_from_pattern(left, source, out);
                    }
                }
            }
            Lang::Go => {
                if kind == "short_var_declaration" || kind == "var_declaration" {
                    Self::extract_identifiers_from_pattern(node, source, out);
                }
            }
            Lang::Java | Lang::Cpp => {
                if kind == "local_variable_declaration" || kind == "declaration" {
                    Self::extract_identifiers_from_pattern(node, source, out);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::collect_locals(language, child, source, out);
        }
    }

    fn extract_identifiers_from_pattern(node: Node, source: &str, out: &mut std::collections::HashSet<String>) {
        if node.kind() == "identifier" {
            let text = crate::symbols::node_text(node, source).to_string();
            out.insert(text);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::extract_identifiers_from_pattern(child, source, out);
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
    fn graph_persistence_round_trip() {
        let src = "fn target() {}\nfn caller() { target(); }\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();
        
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.json");
        
        g.save(&path).unwrap();
        let loaded = KnowledgeGraph::load(&path).unwrap();
        
        assert_eq!(g.symbol_count(), loaded.symbol_count());
        let caller = *g.find_by_name("caller").first().unwrap();
        let target = *g.find_by_name("target").first().unwrap();
        
        let calls = loaded.neighbors(caller, EdgeKind::Calls);
        assert!(calls.contains(&target));
    }

    #[test]
    fn local_variables_are_not_linked_as_globals() {
        let src = "fn global_func() {}\nfn caller() {\n    let global_func = 1;\n    global_func;\n}\n";
        let mut g = KnowledgeGraph::new();
        g.add_file(&PathBuf::from("a.rs"), src, &registry())
            .unwrap();

        let global = *g.find_by_name("global_func").first().unwrap();
        let caller = *g.find_by_name("caller").first().unwrap();

        // The identifier `global_func` inside `caller` matches the local variable,
        // so it should NOT create a reference to the global `global_func`.
        let refs = g.neighbors(caller, EdgeKind::References);
        assert!(!refs.contains(&global), "local variable should not link to global function");
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

    // ---- qualified names (§4) -------------------------------------------

    #[test]
    fn qualified_name_includes_module_path() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "mod api {\n    fn new() {}\n}\n",
            &registry(),
        )
        .unwrap();

        let new_id = *g.find_by_name("new").first().unwrap();
        let qname = g.resolve_qualified_by_id(new_id).unwrap();
        assert!(
            qname.contains("api") && qname.contains("new"),
            "qualified name should include module path, got {qname}"
        );
    }

    #[test]
    fn two_same_named_methods_get_distinct_qnames() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "mod a {\n    fn new() {}\n}\nmod b {\n    fn new() {}\n}\n",
            &registry(),
        )
        .unwrap();

        let ids = g.find_by_name("new");
        assert_eq!(ids.len(), 2, "expected two new() symbols");
        let q1 = g.resolve_qualified_by_id(ids[0]).unwrap();
        let q2 = g.resolve_qualified_by_id(ids[1]).unwrap();
        assert_ne!(q1, q2, "two new() in different modules must have distinct qnames");
    }

    #[test]
    fn resolve_qualified_finds_symbol() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "fn greet() {}\n",
            &registry(),
        )
        .unwrap();

        let greet_id = *g.find_by_name("greet").first().unwrap();
        let qname = g.resolve_qualified_by_id(greet_id).unwrap();
        assert_eq!(g.resolve_qualified(&qname), Some(greet_id));
        assert_eq!(g.resolve_qualified("nonexistent::nope"), None);
    }

    // ---- import resolution (§4) -----------------------------------------

    #[test]
    fn rust_use_import_resolves_to_qualified_symbol() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("helper.rs"),
            "pub fn debug() {}\n",
            &registry(),
        )
        .unwrap();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "use helper::debug;\nfn foo() {\n    debug();\n}\n",
            &registry(),
        )
        .unwrap();

        let foo_id = *g.find_by_name("foo").first().unwrap();
        let calls = g.neighbors(foo_id, EdgeKind::Calls);
        assert!(
            !calls.is_empty(),
            "foo should call debug via import resolution"
        );
        let callee_name = g.symbol(calls[0]).map(|s| s.name.as_str());
        assert_eq!(callee_name, Some("debug"));
    }

    #[test]
    fn python_from_import_resolves() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("math_utils.py"),
            "def add(a, b):\n    return a + b\n",
            &registry(),
        )
        .unwrap();
        g.add_file(
            &PathBuf::from("app.py"),
            "from math_utils import add\ndef run():\n    add(1, 2)\n",
            &registry(),
        )
        .unwrap();

        let run_id = *g.find_by_name("run").first().unwrap();
        let calls = g.neighbors(run_id, EdgeKind::Calls);
        assert!(
            !calls.is_empty(),
            "run should call add via from-import"
        );
    }

    #[test]
    fn js_default_import_resolves() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("utils.js"),
            "export function helper() {}\n",
            &registry(),
        )
        .unwrap();
        g.add_file(
            &PathBuf::from("app.js"),
            "import helper from './utils';\nfunction main() {\n    helper();\n}\n",
            &registry(),
        )
        .unwrap();

        let main_id = *g.find_by_name("main").first().unwrap();
        let calls = g.neighbors(main_id, EdgeKind::Calls);
        assert!(
            !calls.is_empty(),
            "main should call helper via default import"
        );
    }

    #[test]
    fn go_import_resolves() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("fmt.go"),
            "package fmt\nfunc Debug(v interface{}) {}\n",
            &registry(),
        )
        .unwrap();
        g.add_file(
            &PathBuf::from("main.go"),
            "package main\nimport \"fmt\"\nfunc main() {\n    fmt.Debug(nil)\n}\n",
            &registry(),
        )
        .unwrap();

        let main_id = *g.find_by_name("main").first().unwrap();
        let calls = g.neighbors(main_id, EdgeKind::Calls);
        assert!(
            !calls.is_empty(),
            "main should call Debug via Go import"
        );
    }

    // ---- query API (§4) -------------------------------------------------

    #[test]
    fn callers_of_returns_correct_symbols() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "fn callee() {}\nfn a() { callee(); }\nfn b() { callee(); }\n",
            &registry(),
        )
        .unwrap();

        let callee_id = *g.find_by_name("callee").first().unwrap();
        let qname = g.resolve_qualified_by_id(callee_id).unwrap();
        let callers = g.callers_of(&qname);
        assert_eq!(callers.len(), 2, "two functions call callee");
        let names: Vec<_> = callers
            .iter()
            .filter_map(|id| g.symbol(*id).map(|s| s.name.clone()))
            .collect();
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
    }

    #[test]
    fn callees_of_returns_correct_symbols() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "fn x() {}\nfn y() {}\nfn caller() { x(); y(); }\n",
            &registry(),
        )
        .unwrap();

        let caller_id = *g.find_by_name("caller").first().unwrap();
        let qname = g.resolve_qualified_by_id(caller_id).unwrap();
        let callees = g.callees_of(&qname);
        assert_eq!(callees.len(), 2, "caller calls two functions");
        let names: Vec<_> = callees
            .iter()
            .filter_map(|id| g.symbol(*id).map(|s| s.name.clone()))
            .collect();
        assert!(names.contains(&"x".to_string()));
        assert!(names.contains(&"y".to_string()));
    }

    #[test]
    fn references_to_includes_callers() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "fn target() {}\nfn caller() { target(); }\nfn user() { target(); }\n",
            &registry(),
        )
        .unwrap();

        let target_id = *g.find_by_name("target").first().unwrap();
        let qname = g.resolve_qualified_by_id(target_id).unwrap();
        let refs = g.references_to(&qname);
        assert_eq!(refs.len(), 2, "two symbols reference target");
    }

    #[test]
    fn query_on_nonexistent_qname_returns_empty() {
        let g = KnowledgeGraph::new();
        assert!(g.callers_of("no::such::thing").is_empty());
        assert!(g.callees_of("no::such::thing").is_empty());
        assert!(g.references_to("no::such::thing").is_empty());
    }

    #[test]
    fn qualified_names_persist_through_save_load() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("lib.rs"),
            "mod inner {\n    fn helper() {}\n}\n",
            &registry(),
        )
        .unwrap();

        let helper_id = *g.find_by_name("helper").first().unwrap();
        let qname = g.resolve_qualified_by_id(helper_id).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.json");
        g.save(&path).unwrap();
        let loaded = KnowledgeGraph::load(&path).unwrap();

        assert_eq!(loaded.resolve_qualified(&qname), Some(helper_id));
    }

    #[test]
    fn remove_file_cleans_qualified_names() {
        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("a.rs"),
            "fn alpha() {}\n",
            &registry(),
        )
        .unwrap();
        let alpha_id = *g.find_by_name("alpha").first().unwrap();
        let qname = g.resolve_qualified_by_id(alpha_id).unwrap();
        assert!(g.resolve_qualified(&qname).is_some());

        g.remove_file(&PathBuf::from("a.rs"));
        assert!(g.resolve_qualified(&qname).is_none());
    }
}
