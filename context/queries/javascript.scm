; Tree-sitter query for JavaScript symbol extraction.
; The `@symbol` capture anchors on the whole declaration node so the
; recorded range spans the whole definition.

(function_declaration) @symbol

(generator_function_declaration) @symbol

(class_declaration) @symbol

(method_definition) @symbol

(program) @symbol
