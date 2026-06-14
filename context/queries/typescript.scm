; Tree-sitter query for TypeScript symbol extraction.
; The `@symbol` capture anchors on the whole declaration node so the
; recorded range spans the whole definition.

(function_declaration) @symbol

(function_signature) @symbol

(class_declaration) @symbol

(abstract_class_declaration) @symbol

(interface_declaration) @symbol

(enum_declaration) @symbol

(module) @symbol

(namespace_declaration) @symbol
