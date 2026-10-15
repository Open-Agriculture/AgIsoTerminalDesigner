# Desktop CLI

Build with `cargo build --release --bins`, open a project in the desktop editor, and invoke `target/release/aitd`. Use `aitd --help` or a command's `--help` to discover arguments.

Edits apply immediately. Each edit command, or complete command file, creates one undo step in the editor. Project saving remains an editor action. The authenticated local connection currently requires Unix owner-only runtime permissions.

## Inputs and discovery

All inputs use commands, arguments, and text assignments. There is no JSON input mode, JSON request file, or raw-operation escape hatch. JSON-looking text supplied to a text property is literal text. Internal transport serialization is independent of the command interface.

```sh
aitd status
aitd object list --type Button
aitd object get Start --parents --children --macros
aitd schema Button --properties width,height,options
aitd tree 1000
aitd context --scope pool --limit 20
aitd region 1000 --region top --limit 10
aitd colours 6 dark_blue '#06162E'
```

Object targets accept decimal IDs, hexadecimal `0x` IDs, and unambiguous names. Prefix a target with `name:` to force name interpretation, including numeric-looking names. Newly created objects can be addressed by name later in the same command file. Edit results include allocated IDs, changed IDs, transaction identity, and revision. Inspections return bounded results and pagination information; request additional details only when needed. `object list --all` reads pages from the same revision. `object get --style` adds factual descendant style examples.

## Properties, fields, and arrays

```sh
aitd object create NumberVariable Counter
aitd object set Counter value=42
aitd object create StringVariable Caption
aitd object set Caption 'value=123'
aitd object rename Caption 'Display caption'
aitd object set Counter value --value-file number.txt
aitd object set 'Display caption' value --value-file caption.txt
```

Property metadata determines types: numbers accept numeric text, booleans accept `true` or `false`, enums accept their declared names, and text properties preserve literal text. Quotes are shell quoting, not value encoding. Schema discovery shows writable properties, defaults, enum choices, and constraints. Colour properties accept a palette index, colour name, or `#RRGGBB`; RGB resolves to the nearest effective palette entry.

Use field assignments for records and flags. Unspecified fields retain their values. Use dotted paths for nested records and zero-based array indices:

```sh
aitd object field Start options latchable=true
aitd object field Shape points.0 x=12 y=20
aitd object array append Shape points x=30 y=40
aitd object array set Shape points 0 x=10 y=15
aitd object array remove Shape points 0
aitd object array clear Shape points
```

Record appends require all scalar leaf fields advertised by the array element schema. Fields must exist, assignments must be unique, and indices must be in range. Nested array paths are supported, for example `code_planes.0.character_ranges`. JSON records and arrays are rejected as structured values. Binary properties use `object bytes-import TARGET PROPERTY FILE` and `object bytes-export TARGET PROPERTY --output FILE`.

## References and ordered relationships

```sh
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

```text
# changes.aitd
object create NumberVariable Counter
object set Counter value=42
project settings set mask_size=640
```

```sh
aitd --check run --file changes.aitd
aitd run --file changes.aitd
aitd run --file - < changes.aitd
aitd --check object set Counter value=77
```

A command file contains up to 256 edit actions. Commands execute in order on an isolated project copy. Every command must be valid when reached: create referenced objects first, repair incompatibilities before changing VT, and resize/reposition before shrinking their available bounds. Any failure discards the entire batch. Successful batches commit as one undo step; no-ops create no history entry. Checks run the same validation without committing and do not reserve IDs or a revision for later execution.

Command files support single/double quotes, escaped characters, trailing backslash continuations, blank lines, and lines beginning with `#`. They perform no variable expansion, command substitution, or shell execution. Text/image/binary paths resolve relative to the command file; stdin command files resolve paths relative to the current directory. Use text files for multiline values. Global selectors, `--json`, and `--check` belong on `run`, not inside the file. Inspection and output commands cannot be included in an edit batch.

## Settings, validation, and guards

```sh
aitd project settings
aitd project settings set mask_size=640 soft_key_width=80 soft_key_height=96
aitd project settings set vt_version=5 arrangement=bottom physical_key_count=12 keys_per_line=6 navigation_key_count=2
aitd project id-range-set Button 6000 6999
aitd validate
```

Settings use the editor's shared validation: VT2–VT6, positive dimensions, valid softkey counts and layout, and complete nonoverlapping ID ranges. Macro ID ranges for older VT targets must fit their supported limits. Dimension changes do not scale objects and cannot introduce or worsen overflow. Ordinary edits may repair invalid imported content while preserving unchanged existing errors. Changing VT additionally requires compatible materialized content and successful encoding for the target version. All fields in one settings command are checked together.

Use `--instance ID` or `AG_ISO_EDITOR_INSTANCE_ID` when multiple editors run. Project identity and revision are resolved once per invocation and guarded on requests. For a longer agent workflow, obtain identity/revision from `status --json`, then pass `--project-instance ID --revision NUMBER` with the edit. A stale guard rejects the request; inspect again before retrying. Draft/proposal commands, candidate views, and persistent draft selection have been removed.

## Output and exports

Readable text is the default. `--json` selects machine-readable **output only**; successes go to stdout and failures to stderr. Editor responses use `success`, `editor_instance_id`, `project_instance_id`, `revision`, and `result` or `error`.

```sh
aitd preview --mask 1000 --output build/layout.png
aitd export bundle --iop build/pool.iop --header build/pool.h
aitd graphic export ASSET_ID --output build/image.png
aitd completions bash > aitd-completion.bash
```

Previews are schematic geometry images, not screenshots of the full VT renderer. Visual-quality heuristics are not provided; `validate` reports project validity. Exports use the live materialized project. Existing output files require `--force`; bundle artifacts are generated and staged before publishing. C header export rejects colliding or reserved identifiers.

Exit codes: `0` success, `1` other failure, `2` invalid usage, `3` missing project/editor or connection failure, `4` stale project/revision, and `5` validation failure.
