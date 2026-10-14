# Designer settings

The top bar contains **File**, **Project**, and **View**, followed by the project
name, unsaved-change marker, and Undo/Redo. File keeps the existing project and
ISOBUS import/export workflows. Project contains Add Object and Project Settings.

## Project dimensions

Project Settings edits the square data/alarm-mask design size and softkey
designator dimensions, arrangement, physical key count, keys per column or row,
and navigation-key count. Apply validates the configuration and records one undo
action; Cancel discards the draft. Dimensions are positive 16-bit integers in
design pixels. Changing them changes design bounds without scaling objects.
Mask and softkey-content reductions cannot introduce or increase placement
overflow.

New projects use a 500 × 500 mask, 60 × 60 softkeys, 12 physical keys on the
right in two columns of six, two navigation keys, and VT6. Existing projects
retain their mask, softkey layout and VT version. Missing softkey dimensions are
inferred per axis from Key contents, with a 60 × 60 baseline; missing layout
settings use the new-project defaults.
Project Settings selects VT2 through VT6. Apply checks compatibility before
changing the version and dimensions together as one undo action. Cancel discards
the draft. Incompatible changes report the affected objects and attributes;
objects are never silently converted or deleted.

Object creation, attribute choices, references, macro events, and previews use
the selected target. Checks cover the designer's existing objects, including
version-specific flags, fonts, encodings, macro commands, and containment rules.
Controls and choices unavailable for the target VT are hidden. Properties and
Add Object show a collapsed **Unavailable in VT…** note listing relevant features
and their supported versions. Existing incompatible values remain visible with
warnings and controls for repair; hiding never changes stored values. Planned
Project Settings placeholders remain visible and are not assigned a VT version.
Optional hardware capabilities remain separate from specification compatibility.
Missing object renderers are not implemented by choosing a newer VT version.

Raw IOP import asks for the encoding's target version because the file does not
contain that information. Malformed or truncated input is rejected without
replacing the open project. Structurally valid incompatible content opens for
repair, as do existing incompatible projects. Edits can retain existing violations
but cannot introduce or change them; fixing one issue at a time is supported.
IOP export requires a compatible materialized pool, including live components.

New saves use `.aitp` format v2 with version-independent authoring objects. This
retains Unicode text, local fallback values, and incompatible repair drafts.
Format v1 remains readable using its historical codec rather than its saved target
version. Unknown formats or targets are reported explicitly. Older applications
that only support format v1 cannot open v2 saves.

VT2/VT3 exports encode edited strings with their consumers' 8-bit font character
sets. Unrepresentable text and conflicting encodings for a shared String Variable
are reported. Unreferenced edited variables use ISO-8859-1. Imported bytes remain
unchanged; unused local strings of variable-backed fields are omitted from legacy
IOP exports. VT4 and later support UTF-16LE. String lengths are checked in encoded
bytes, including the 255-byte limit of variables consumed by Input String fields.
VT5 and later support extended macro IDs; earlier targets limit macros to 255.

Preview uses VT4+ button and Boolean rules and pixel-based text justification.
VT2/VT3 previews use character-cell justification with top alignment; where the
older standard leaves presentation undefined, the editor uses its deterministic
control-character handling. The preview is an authoring aid, not runtime VT emulation.

The editor shows advisories rather than rejecting designs below terminal minima.
ISO 11783-6:2018 §4.5.2 requires a square terminal data-mask area of at least
200 × 200 pixels before VT6 and 480 × 480 for VT6. Section 4.5.3.1 specifies
softkey designator minima of 60 × 32 before VT6 and 60 × 60 for VT6. These are
terminal capabilities, not a reason to reject existing or scaled design coordinates.
The mask size also applies to Alarm Masks; softkeys occupy a separate area.

## Application preferences

View offers System, Light and Dark themes. The default follows the operating
system. Themes style editor controls and canvas guides without changing
object-pool colours.

Objects, Graphics and Components initially share a tab group on the left, with
Canvas in the centre, Properties on the right and a compact Selection Ancestry
pane beneath Canvas. The desktop version also offers Proposal. Drag a pane's
tab to reorder tabs, split the workspace or float the pane inside the application.
Resize splits and floating panes to suit your workspace. Ancestry's details button
expands the existing relationship tree.

View → Panels hides, reopens and activates individual panes, including Canvas.
New proposals reveal their review pane. Reset Panel Layout restores the default
arrangement while preserving theme and ID format. Layout, active tabs and floating
window positions persist locally across project changes and application restarts;
they are not stored in `.aitp` projects. Invalid saved layouts fall back to defaults.

Selected object rows have a continuous highlight spanning the tree width,
including indentation, so the selection remains visible in deeply nested trees.

View → Object IDs and Project Settings → Authoring control whether numeric IDs
appear alongside object names. IDs are hidden by default. The decimal or
hexadecimal preference controls their display; hexadecimal displays IDs as
`0x00AF`. Labels, reference selectors, ID search and editable ID fields use the
selected format. Unprefixed numeric ID input uses the selected radix; `0x` or
`0X` always means hexadecimal. Existing ID uniqueness, range and null-reference
rules still apply. Explicit object names, generated names, serialized IDs and
exports are unchanged.

Automatic ID ranges are project settings. Every object type has an inclusive
range within `0–65534`, and ranges cannot overlap. Defaults assign each ordinary
type its type-number block (`type × 1000` through `+999`), Working Set objects
`256–999`, and macros `0–255`. VT2–VT4 projects keep macro IDs within `0–255`;
VT5 and later projects may configure a larger macro range. Applying range edits
validates them with the other project settings and records one undoable change.
Changing a range affects future automatic assignments and never renumbers
existing objects. Reset to defaults replaces the draft ranges; Cancel discards
them.

The object inspector shows the assigned ID separately from its name. **Override
ID** permits an unused ID outside the automatic range, subject to the target VT
version. **Use automatic ID** chooses a fresh free ID in the type's range.
Overrides persist in the project and participate in undo/redo.

**Reassign object IDs…** previews names, old/new IDs and preserved overrides,
including compiled component nodes. Manual overrides are preserved unless
explicitly included. Errors in allocation, references or macro packets block
reassignment before mutation. Applying a valid preview updates references in one
undoable change. Regenerate external ECU ID headers after reassignment.

Theme, workspace layout, ID format and the Show object IDs preference persist
locally across restarts and project changes. They do not dirty a project or
enter its undo history. Canvas zoom, pan and snapping retain their existing
controls.

## Shared attributes

Project Settings → Shared resources → **Manage Shared Attributes…** opens a
searchable manager for ordinary Font, Line, and Fill Attributes objects, including
unused objects. Create, rename, duplicate, delete, and edit them using the existing
attribute controls. Assign shared attributes through the reference selectors in
individual object editors. Editing an attribute updates every object using it.

Changes apply immediately and enter the normal project undo history. Opening the
manager closes Project Settings and discards its unapplied configuration draft.
Duplicating creates an independent attribute with a unique name and a fresh
automatic ID from the configured range; existing consumers retain their references.
The manager lists consumers, including component instances and editor-only allowed
targets. Click a consumer to open its Properties. Referenced attributes cannot be
deleted. If component usage cannot be checked, deletion is disabled with an
explanation. Generated component attributes remain managed through Components.

Shared attributes are normal object-pool resources and retain their existing
project persistence and IOP export behavior. Colour styles, creation presets, and
reusable graphic defaults remain future settings.

## Planned configuration

The remaining settings below have disabled controls with **Not configured**
explanations. They do not have serialized configuration or affect the object pool.

| Section | Planned settings | Required capability |
| --- | --- | --- |
| Compatibility | Colour capability, supported fonts/styles, input capabilities, terminal object support | Terminal capability profiles |
| Languages | Supported/default languages, translation resources | Language resource management and variant generation |
| Authoring | Naming conventions | Configurable naming policies |
| Export | Export groups, language variants, generated-code options | Export profiles and variant/code generation |
| Shared resources | Colour styles, font presets, reusable graphic defaults | Project-wide resource defaults and styles |

Existing graphics, components and object attribute editors remain available.
Per-object colours, font attributes, graphics, active masks and Working Set
language codes belong to their resources/objects, rather than duplicated project
properties. Runtime ECU identity, CAN configuration and transfer timeouts belong
to the client application and remain outside the designer settings.

## References

- ISO 11783-6:2018 §§4.1.1, 4.5.2–4.5.3 and Annex D: terminal version, dimensions,
  softkeys, fonts, graphics and other capabilities. The user-supplied Part 6 PDF
  was used to check the requirements; see the [ISO catalogue entry](https://www.iso.org/standard/71173.html).
- [Epec integration manual](https://extranet.epec.fi/Public/Manuals/EPEC_Programming_And_Libraries/projecttopics/topic100343.htm): ISO-Designer hardware properties and matching client design dimensions.
- [Bucher ISO-Designer](https://www.bucherautomation.com/en/iso-designer/sw10133): specification selection, configurable workspace, multilingual resources and export workflows.
- [OSB VTdesigner](https://osb-connagtive.com/isobus-products/vt-designer/): multilingual projects and IOP/C/header/XML export options.
- [ISOBUS Studio](https://www.isobus-studio.com/en/) and its [feature list](https://www.isobus-studio.com/en/pricing): theme controls, ID-range management, colour styles, translations and export grouping. Several features are listed as planned or coming soon; they are inspiration rather than proof of a working configuration UI.
