# Removal Workflow

- Prefer deletion over preservation. Do not re-add, shim, or locally recreate removed behavior just to satisfy the compiler.
- Do not use the plan tool.
- Removal targets are global, not scoped to the current slice. If removing one target exposes or depends on another target, remove the other target too; do not preserve it just because the immediate edit was "about" something else.
- For any removal target, remove the whole surface: request/response protocol variants, processors, config schema, re-exports, tests, docs, helper modules, and dependent feature/config glue. Do not leave inert fallbacks like `false`, `None`, empty vectors, or empty match arms.
- Adapt retained code locally as needed, but only after deleting every touched removal-target branch. Never repair removed-target logic into a compiling stub.
- Treat `.removal/REMOVAL_PLAN.md` as the authoritative removal-target list. Do not maintain a partial target list here; if the plan says a surface is removed, remove every touched branch of that surface transitively.
- Remove removed-target paths outright; do not replace removed data or behavior with placeholders like `Vec::new()` or equivalent inert fallbacks.
- Use `codemap` to inspect symbols and ownership before broad edits.
- Use lexical search (`rg`) to find every remaining reference to a removed surface.
- Use `.removal/rust-prune-query/rust-prune-query` with focused `.scm` queries for mechanical Rust deletions; dry-run first, inspect captures, then write.
- Leave compiler checks until the end of a coherent removal slice. Compiler errors are not instructions to preserve removed abstractions.
- Only preserve or add behavior when the target product explicitly wants that retained behavior.
- Record uncertain semantic follow-ups in `.removal/REMOVAL_PLAN_SEMANTIC_ONLY.md`.
