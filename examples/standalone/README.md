# Standalone CLI example

This example creates a small valid VT project, applies two separate `.aitcmd` files, validates and saves the project, then exports the IOP and C object-ID header twice and compares the results. It uses only the `aitd` CLI; no desktop editor or display is required.

Build `aitd` first, then run:

```bash
cargo build --release --bin aitd
./examples/standalone/create-and-export.sh
```

The script finds `target/release/aitd` from the repository root. Set `AITD=/path/to/aitd` to use another binary. All generated project and export files live in a temporary directory that the script removes when it exits. The command files are kept in `resources/` and each invocation in the script shares the same temporary project session.
