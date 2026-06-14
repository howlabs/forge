; Tree-sitter query for Python symbol extraction.
; The `@symbol` capture anchors on the whole declaration node so the
; recorded range spans the whole definition.

(function_definition) @symbol

(class_definition) @symbol

(decorated_definition) @symbol

(module) @symbol
