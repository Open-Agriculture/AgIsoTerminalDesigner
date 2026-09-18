# Phase 5.6 — Semantic visual styling

The model chooses visual intent. The harness chooses VT encoding. No changes were
made to the operation executor, history format, apply/undo semantics, model routing,
or provider replay architecture.

## Changed files

New implementation: `src/effective_colour.rs`, `src/ai/visual_intent.rs`,
`src/ai/icon_recipes.rs`, `src/ai/visual_diagnostics.rs`.
Integration: `src/lib.rs`, `src/object_rendering.rs`, `src/ai/mod.rs`,
`backend.rs`, `schema.rs`, `snapshot.rs`, `design_inspection.rs`, `tools.rs`,
`wire.rs`, `task_state.rs`, `ui.rs`, `design_analysis.rs`, `preview.rs`.
Tests: `tests/semantic_visual_styling.rs` and the existing tool/UI test modules.
This document records the implementation and validation.

Pre-existing changes in `src/object_configuring.rs` and `src/isobus_colour.rs`
were preserved. The existing basic colour labels in `isobus_colour` are reused;
there is no second AI RGB table.

## Authoritative resolution and effective palettes

`effective_colour::colour(pool, index)` is the shared entry point for every
indexed colour in object rendering, including PictureGraphic decoding.
It reads the ColourMap and ColourPalette explicitly referenced by
WorkingSetSpecialControls. Unreferenced palette objects have no effect.
Mapping precedes palette lookup; missing controls/entries fall back to the pool's
mapping. For malformed pools containing multiple controls, lowest object ID wins
in both rendering and AI resolution. This is static project rendering; runtime
terminal commands are not simulated.

Default RGB values come from AgIsoStack's existing palette. Basic names reuse the
existing editor labels. The canonical aliases are `dark_blue` (navy), `gray`, and
`light_gray` (silver). A custom RGB without an exact basic-name match is named by
its hex value, avoiding a misleading claim of an exact named colour.

Picture texture hashes include the effective palette, so changing a palette/map
invalidates cached raster colours even if picture bytes did not change.

## AI colour DTO and normalization

`AiColourValue` is an AI-boundary type: integer index, `{ "name": "cyan" }`, or
`{ "rgb": "#06162E" }`. It does not alter the domain's u8 properties.
The compiler discovers types of existing and newly created objects, uses
PropertySemantic::Colour, and replaces semantic values with numeric SetProperty
values before ordinary transaction preparation. Other properties remain untouched.
Explicit raw indices are preserved exactly, including duplicate-RGB indices.

Nearest matching uses squared Euclidean RGB distance and lowest index for ties.
Aliases normalize deterministically: navy/dark navy/dark blue/deep blue,
cyan/aqua, gray/grey, and light gray/light grey/silver.
Unsupported names return `AI_COLOUR_UNKNOWN` with supported names; malformed RGB
returns `AI_COLOUR_INVALID`. Resolution metadata includes requested and actual
values, squared RGB distance (zero means exact), and palette source.

If the same transaction explicitly changes palette configuration, a disposable
candidate is prepared using the normal executor and semantic intents are compiled
against that candidate's final palette. The final primitive transaction still
passes normal preparation. No palette is created automatically.

## Enriched reads and schemas

All descriptor-declared colours in get_design_context, get_objects,
inspect_design_region, and nearby-style summaries use:

```json
{"index":11,"name":"cyan","rgb":"#00FFFF"}
```

Provider schemas advertise `value_forms: ["index", "name", "rgb"]` and examples.
The single read-only `resolve_colours` tool accepts 1–16 queries. No full palette
is appended to every prompt. Proposal results and debug tool traces include
normalization metadata and an actual-colour summary bounded to 64 objects with a
truncation flag. RenderedPreview also carries this summary. Existing binary-read
sanitization keeps image bytes out of snapshots.

## Structured theme and consistency

finish_inspection accepts optional theme intent:

```json
{
  "required_object_types": ["DataMask", "PictureGraphic"],
  "theme": {
    "background": {"name":"dark_blue"},
    "primary_text": {"name":"white"},
    "secondary_text": null,
    "accent": {"name":"cyan"},
    "danger": null
  }
}
```

Strict provider schemas require nullable role fields; the provider-neutral parser
also accepts omitted roles. AiTaskState retains both requested and resolved theme,
which is included in subsequent provider context and resolved again on project
revision changes or candidate palette changes.

Before staging, DataMask/AlarmMask background edits are checked against theme
background. SetProperty can optionally annotate `theme_role` for an explicit
background/primary_text/secondary_text/accent/danger association. That annotation
compiles away. A mismatch returns `STYLE_INTENT_MISMATCH` with expected/actual
resolved colours and object context. Equivalent RGB at duplicate indices is valid.
Unannotated button/status/icon accent variations are not rejected. Comparisons use
the final mask property, so a correction can override an earlier operation.

## Contrast and shared-style diagnostics

Contrast uses WCAG relative luminance and contrast ratio as an internal readability
heuristic, not an ISO 11783 compliance assertion. Text backgrounds follow the
placement path through transparent ancestors, resolving FontAttributes references.
Diagnostics are warnings and never change requested colours:
`LOW_TEXT_CONTRAST`, `LOW_BUTTON_CONTRAST`, `LOW_STATUS_CONTRAST`.
Status-specific labeling is a conservative name-based heuristic; other labels
still receive text contrast checks. Structured context records foreground,
background, ratio, threshold, object/font/background IDs, and placement path.

Thresholds default to 4.5:1 and are configurable through
`analyze_design_with_thresholds` / `ContrastThresholds`. Hover, pressed-state
shading and runtime flashing are outside this static check.

Before a candidate is staged, shared FontAttributes/LineAttributes/FillAttributes
colour edits report `SHARED_STYLE_SIDE_EFFECT` with direct usage count, affected
mask count, and up to 16 affected IDs. Guidance calls for dedicated attributes and
a redirected reference for local edits. Global shared edits remain allowed.
Automatic cloning is deliberately not introduced; the helper was optional.

## Icon recipes

CreateIcon is an AI-only operation with icon, name, width, height, foreground,
transparent, optional background, optional object_id, and optional parent/x/y.
Thirteen normalized-grid raster recipes are supported: wifi, lock, unlock, check,
warning, info, disconnect, signal_low, signal_medium, signal_high, refresh, search,
settings. Dimensions are bounded to 8–128 pixels per axis.

The compiler generates an indexed EightBit PictureGraphic and ordinary
CreateObject/SetProperty/AddChild operations. Transparent icons choose a colour key
whose rendered RGB differs from the foreground; a palette with no distinct key
returns a diagnostic. The model supplies no bitmap bytes. There is no image model,
SVG parser, extra executor, or persistent high-level intent in history.

## Explicit regression and results

Request: “Make the screen more modern, for example by changing colours and adding
icons.” Previously a navy/cyan plan could produce background_colour = 6 without
knowing the rendered colour. Now background=dark_blue, accent=cyan,
primary_text=white resolve through the effective renderer palette.

This repository's actual default palette resolves those roles to indices
15 (#000099), 11 (#00FFFF), and 1 (#FFFFFF). Index 6 is olive (#999900), not navy.
The requested custom-palette fixture places navy at 4 and yellow at 6; it resolves
navy to 4 and rejects a background edit to 6 under the navy theme. A custom map
moves navy to 20, and semantic compilation follows that mapping, including when
the map and style change in the same proposal.

Automated coverage includes the scripted provider inspection/proposal/repair loop,
Luna/Fast remaining selected, no canonical mutation, explicit indices, stable RGB
nearest matching, unknown names, all required read formats, explicit theme roles,
valid red accent variation, white-on-white and white-on-navy contrast, button/status
contrast and configurable thresholds, shared font impact, existing style reuse,
all 13 icon recipes through transaction validation and actual egui texture rendering,
and custom-palette rendering/cache invalidation.

The existing transport preview remains a geometry-oriented approximation; the
interactive preview uses the actual renderer. Resolved colour context accompanies
review so exact colours do not have to be inferred from that image. Live model
output was not exercised; the regression uses a deterministic scripted provider.

Validation: `cargo check` passed; `cargo test` passed (66 tests, zero failures,
one existing ignored doctest); `git diff --check` passed. Compiler warnings in
existing code remain. No live provider credentials or network access were needed.
