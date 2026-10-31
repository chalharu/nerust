# Schema v2 markup

`parse(source, "yaml" | "yml" | "json", filename) -> Result<Document, Error>`
is the shared strict parser. `schema() -> serde_json::Value` exports JSON Schema;
`fingerprint(source) -> String` returns a stable, 16-digit lowercase FNV-1a hash
of exact UTF-8 bytes (not a cryptographic signature). `Document::validate()` also
checks manually constructed ASTs. Public AST types are in `src/lib.rs`.

Input limits: 1 MiB, nesting depth 64, 16,384 value nodes, and a separate 16,384
expanded-control budget. Expansion rejects missing/recursive templates, including
unused templates. Every object rejects unknown fields and every mapping rejects
duplicate keys. YAML anchors, aliases, merge keys and custom/standard explicit
tags are rejected lexically, not by searching inside quoted/block literals.
The dependency's official YAML tokenizer distinguishes these tokens, including
multiline scalars and document markers. Only `true`/`false` infer as booleans;
YAML 1.1 spellings such as `n`, `yes` and `on` remain strings.
No arbitrary Rust strings are evaluated. Parser syntax errors retain their
line/column and filename; structural errors retain the closest available YAML
path. Not every semantic error can be mapped to an exact line after lowering.

See `../macros/README.md` for precise executable syntax. Schema export helps
schema-aware editors; this prototype does **not** implement a complete editor LSP.