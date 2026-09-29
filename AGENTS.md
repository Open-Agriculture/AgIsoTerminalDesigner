# Repository guide for Codex agents

## Project shape

- Rust 2021 desktop and WebAssembly app built with `egui`/`eframe`; `src/main.rs` owns the `DesignerApp` UI and application lifecycle.
- `src/editor_project.rs` is the canonical editing model. It coordinates the AgIsoStack `ObjectPool`, components, graphics, metadata, revision tracking, and undo/redo history.
- UI and domain concerns are split across `src/object_picker.rs`, `src/object_configuring.rs`, `src/object_properties.rs`, and `src/object_rendering.rs`.
- `src/operations/` contains transactional edits and history; `src/pool_validation.rs` validates pools.
- `src/project_file.rs` handles `.aitp` JSON projects and IOP object-pool bytes. Keep project persistence and pool serialization behavior aligned.
- `src/components/`, `src/graphics/`, and `src/ai/` contain component editing, graphic assets/rendering, and the desktop AI design assistant.
- Tests live in `src/` unit tests and `tests/` integration tests. The AgIsoStack dependency is a sibling checkout at `../AgIsoStack-rs`.

## Working rules

- Treat ISO 11783 semantics and valid object-pool structure as correctness requirements. Follow behavior established by the codebase and AgIsoStack; do not invent protocol rules or create invalid object references.
- Keep edits consistent with module boundaries and Rust style. `EditorProject` owns authoritative project state; use its operations for mutations so validation, dirty state, and undo/redo stay coherent. Distinguish committed project data from UI drafts, caches, and selection state, and keep those states synchronized across relevant UI transitions.
- Preserve both native and `wasm32` builds when changing shared code; gate native-only APIs with the existing target cfgs.
- Preserve `.aitp` compatibility and IOP handling. When changing persistence, inspect load/save paths and validate round trips where practical.
- Before handing off Rust changes, run the relevant checks when the environment supports them. CI commands include:
  - `cargo check --lib --all-features`
  - `cargo check --lib --all-features --target=wasm32-unknown-unknown`
  - `cargo test --lib` (CI's library test command; integration tests are also present under `tests/`)
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features`
- For the full local test suite, use `cargo test`; a wasm check requires the `wasm32-unknown-unknown` target. Native builds may require platform GUI libraries.

## Delegation

For substantial tasks, start with the built-in `explorer` (default Luna) to map relevant files, symbols, tests, and existing behavior. Use `architect` selectively when state ownership or cross-module design needs investigation; ask it to report interfaces and risks. Assign scoped application changes to `app_worker`, validation and round-trip checks to `test_worker`, and an independent review of the integrated diff to `reviewer`. Give each task relevant files and a clear handoff. The primary agent coordinates overlap, examines the final diff, and runs final verification.
