# Standalone CLI

`aitd` is a standalone command-line editor for `.aitp` projects. It does not need the desktop editor, a display, or a background service. Build the CLI with `cargo build --release --bin aitd`, then invoke `target/release/aitd` or add `target/release` to `PATH`. Use `aitd --help` and `aitd COMMAND --help` for the installed command syntax.

Commands run from the same calling shell share a temporary working project, even when the shell changes directories. A new shell, terminal, or separately launched Bash script gets its own session. Sessions are isolated by the caller's process identity and are locked while each command runs. Successful edits update the working copy atomically. The original project file changes only when you run `project save`.

Sessions use the surviving parent process. `exec aitd ...` replaces the shell and uses its parent’s session. For `bash -c` one-liners, append `; :` to keep Bash from replacing itself with the final command.

```bash
aitd project load machine.aitp
aitd run --file resources/labels.aitcmd
aitd object create OutputString StatusLabel --id 11000
aitd object set StatusLabel value=READY
aitd validate
aitd project save machine.aitp
```

See [the runnable standalone example](../examples/standalone/README.md) for a Bash workflow that creates a project, applies separate `.aitcmd` files, validates, saves, and compares repeat exports without opening the editor.

## Project lifecycle and saving

```bash
aitd project new
aitd project load machine.aitp
aitd project close
aitd project close --discard
aitd project save machine.aitp
aitd project save release/machine.aitp --force
aitd project save draft.aitp --skip-validation
aitd backups list
aitd backups list --project machine.aitp
```

`project new` starts an empty authoring project. Loading another project or closing a changed project requires `--discard`; a successful save updates the saved baseline and leaves the project loaded. Saving to the loaded path does not need `--force`. Saving over a different existing path does. By default, `project save` runs validation first: errors prevent the save, while warnings are allowed. `--skip-validation` permits saving an authoring project with validation errors; serialization, filesystem, backup, and overwrite checks still apply. A save is staged beside its destination and atomically replaces it only after all checks succeed. If the loaded source was changed outside `aitd` since load or its last save, the save is rejected so external work is not overwritten.

Before an existing project is overwritten, its exact bytes are retained as a uniquely named `.aitp` backup under the operating system temporary directory. Backups have metadata with their creation time and original absolute path. The shared backup store keeps backups for 24 hours when space permits and has a 1,000,000,000-byte total limit, including metadata; when needed it removes the oldest backups first. A backup larger than the entire limit, or a backup-store failure, prevents the save. Temporary-directory cleanup performed by the operating system may remove backups earlier. Use `backups list` to find a backup, then load it and use ordinary project save commands to restore it. Save output includes the backup path when one was created.

## Inputs and discovery

All inputs use commands, arguments, and text assignments. JSON is supported for output only; JSON-looking text assigned to a property is literal text.

```bash
aitd status
aitd object list --type Button
aitd object get Start --parents --children --macros
aitd schema Button --properties width,height,options
aitd tree 1000
aitd context --scope pool --limit 20
aitd region 1000 --region top --limit 10
aitd colours 6 dark_blue '#06162E'
aitd capabilities
aitd completions bash > aitd-completion.bash
```

Object targets accept decimal IDs, hexadecimal `0x` IDs, and unambiguous names. Prefix a target with `name:` to force name interpretation, including for numeric-looking names. Newly created objects can be addressed by name later in the same command or command file. `object create TYPE NAME --id ID` uses the same explicit object-ID validation as the other ID editing commands. Inspections return bounded results and pagination information; request additional details only when needed. `object list --all` reads all pages, and `object get --style` adds factual descendant style examples.

## Properties, fields, and arrays

```bash
aitd object create NumberVariable Counter --id 21001
aitd object set Counter value=42
aitd object create StringVariable Caption --id 22001
aitd object set Caption 'value=123'
aitd object rename Caption 'Display caption'
```

Property metadata determines types: numbers accept numeric text, booleans accept `true` or `false`, enums accept their declared names, and text properties preserve literal text. Quotes are shell quoting, not value encoding. Schema discovery shows writable properties, defaults, enum choices, and constraints. Colour properties accept a palette index, colour name, or `#RRGGBB`; RGB resolves to the nearest effective palette entry.

Use field assignments for records and flags. Unspecified fields retain their values. Use dotted paths for nested records and zero-based array indices:

```bash
aitd object field Start options latchable=true
aitd object field Shape points.0 x=12 y=20
aitd object array append Shape points x=30 y=40
aitd object array set Shape points 0 x=10 y=15
aitd object array remove Shape points 0
aitd object array clear Shape points
```

Record appends require all scalar leaf fields advertised by the array element schema. Fields must exist, assignments must be unique, and indices must be in range. Nested array paths are supported, for example `code_planes.0.character_ranges`. JSON records and arrays are rejected as structured values. Binary properties use `object bytes-import TARGET PROPERTY FILE` and `object bytes-export TARGET PROPERTY --output FILE`.

## References and ordered relationships

```bash
aitd reference set Label font_attributes TextFont
aitd reference clear Label variable_reference
aitd placement add Mask Start 20 30
aitd placement move Mask Start 40 30
aitd placement remove Mask Start
aitd list insert Choices 0 Counter
aitd list remove Choices 0
aitd event bind Start OnKeyPress=Pressed
aitd event unbind Start OnKeyPress
```

Incremental edits preserve other entries. Use `--occurrence INDEX` to disambiguate repeated placements of the same child (zero-based among matching children). Complete replacement uses `list set`, `pointer set`, or `event set` with positional targets or `EVENT=MACRO` bindings. `null` denotes an empty nullable reference/list slot. Reordering commands require every existing entry exactly once. Label commands accept `id=TARGET`, `string=TARGET`, `graphic=TARGET`, and `font=NUMBER`; use `label --help` for forms.

Components and graphics use their own command groups. `component capture ROOT NAME` creates a reusable definition; `component instantiate ID PARENT X Y` places an instance. `component node-set`, `component override`, and parameter defaults use the same metadata-based value parsing as object properties. Import images with `graphic import FILE --name NAME`. Use `component --help`, `instance --help`, and `graphic --help` for the remaining commands.

## Atomic command files and checks

Command files use the `.aitcmd` extension and contain literal edit commands, one per line:

```text
# resources/labels.aitcmd
object create NumberVariable Counter --id 21001
object set Counter value=42
project settings set mask_size=640
```

```bash
aitd --check run --file resources/labels.aitcmd
aitd run --file resources/labels.aitcmd
aitd run --file - < resources/labels.aitcmd
aitd --check object set Counter value=77
```

A file contains up to 256 edit actions. Commands execute in order on an isolated working-copy candidate. Every command must be valid when reached: create referenced objects first, repair incompatibilities before changing VT, and resize/reposition before shrinking their available bounds. Any failure discards the entire batch. A successful batch updates the working copy atomically. `--check` runs the same checks without changing the working project.

Command files support single/double quotes, escaped characters, trailing backslash continuations, blank lines, and lines beginning with `#`. They perform no variable expansion, command substitution, or shell execution. Image and binary resource paths resolve relative to the command file; stdin command files resolve paths relative to the current directory. `--json` and `--check` belong on `run`, not inside the file. Inspection and output commands cannot be included in an edit batch. A Bash script can coordinate multiple `.aitcmd` files and individual CLI commands; the 256-action limit applies to each file independently.

## Settings and validation

```bash
aitd project settings
aitd project settings set mask_size=640 soft_key_width=80 soft_key_height=96
aitd project settings set vt_version=5 arrangement=bottom physical_key_count=12 keys_per_line=6 navigation_key_count=2
aitd project id-range-set Button 6000 6999
aitd validate
```

Settings use shared project validation: VT2–VT6, positive dimensions, valid softkey counts and layout, and complete nonoverlapping ID ranges. Macro ID ranges for older VT targets must fit their supported limits. Dimension changes do not scale objects and cannot introduce or worsen overflow. Ordinary edits may repair invalid imported content while preserving unchanged existing errors. Changing VT additionally requires compatible materialized content and successful encoding for the target version. All fields in one settings command are checked together. Validation reports errors and nonblocking warnings, including `TEXT_CLIPPED` for fixed-width Output Strings whose current decoded text clips horizontally or vertically; valid wrapping is not flagged.

## Output and exports

Readable text is the default. `--json` selects machine-readable output; successes go to stdout and failures to stderr. Output describes the standalone project session and contains no editor identity or transport fields.

```bash
aitd validate --json
aitd preview --mask 1000 --output build/layout.png
aitd export bundle --iop build/pool.iop --header build/pool.h
aitd graphic export ASSET_ID --output build/image.png
```

Previews are schematic geometry images, not screenshots of the full VT renderer. Binary export applies its own target compatibility checks, independently of save-time validation. Existing output files require `--force`; bundle artifacts are generated and staged before publishing. C header export rejects colliding or reserved identifiers.

The backup listing includes timestamps, original project paths, backup paths, and sizes. Exit codes are `0` for success, `1` for other failures, `2` for invalid usage, `3` for inspection/edit commands without a loaded project, `4` for a changed source, and `5` for validation failure.
