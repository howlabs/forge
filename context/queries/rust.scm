; Tree-sitter query for Rust symbol extraction.
; The `@symbol` capture anchors on the whole declaration node, so
; tree-sitter matches the (function_item) / (struct_item) / etc.
; rather than its inner (identifier) child.  This makes the recorded
; range span the whole definition, which the knowledge graph relies
; on for `Contains` resolution.

(function_item) @symbol

(struct_item) @symbol

(enum_item) @symbol

(trait_item) @symbol

(impl_item) @symbol

(mod_item) @symbol
