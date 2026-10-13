# Designer settings

The top bar contains **File**, **Project**, and **View**, followed by the project
name, unsaved-change marker, and Undo/Redo. File keeps the existing project and
ISOBUS import/export workflows. Project contains Add Object and Project Settings.

## Project dimensions

Project Settings edits the square data/alarm-mask design size and the softkey
designator width and height with numeric inputs. Apply validates all dimensions
and records one undo action; Cancel discards the dimension draft. Dimensions are
positive 16-bit integers in design pixels. Changing them changes design bounds,
without scaling objects. Mask reductions cannot introduce or increase placement
overflow. Softkey dimensions are saved metadata; their canvas preview is pending.

New projects use a 500 × 500 mask, 60 × 60 softkeys, and VT6. Existing projects
retain their mask and VT version. If softkey dimensions are missing, the editor
infers each axis independently from Key contents, with a 60 × 60 baseline.
The saved VT version is read-only until version switching has compatibility checks.

The editor shows advisories rather than rejecting designs below terminal minima.
ISO 11783-6:2018 §4.5.2 requires a square terminal data-mask area of at least
200 × 200 pixels before VT6 and 480 × 480 for VT6. Section 4.5.3.1 specifies
softkey designator minima of 60 × 32 before VT6 and 60 × 60 for VT6. These are
terminal capabilities, not a reason to reject existing or scaled design coordinates.
The mask size also applies to Alarm Masks; softkeys occupy a separate area.

## Application preferences

View offers System, Light and Dark themes and individual visibility controls for
Objects, Graphics, Components, Properties and Selection Ancestry. The desktop
version also offers Proposal. Graphics and Components share a sidebar; hiding
both removes it. New proposals reopen their review window. Reset Panel Layout
restores visible panels and their initial layout, preserving theme and ID format.
Themes change editor chrome, not object-pool colours.

View → Object IDs and Project Settings → Authoring edit the same application
preference. Decimal is the default; hexadecimal displays IDs as `0x00AF`.
Labels, reference selectors, ID search and editable ID fields use the selected
format. Unprefixed numeric ID input uses the selected radix; `0x` or `0X` always
means hexadecimal. Existing ID uniqueness, range and null-reference rules still
apply. Explicit object names, generated names, serialized IDs and exports are unchanged.

Theme, panel visibility and ID format persist locally across restarts and project
changes. They do not dirty a project or enter its undo history. Canvas zoom, pan
and snapping retain their existing controls.

## Planned configuration

These settings have disabled controls with **Not configured** explanations.
They do not have serialized configuration or affect the object pool yet.

| Section | Planned settings | Required capability |
| --- | --- | --- |
| Compatibility | VT version selection, colour capability, supported fonts/styles, input capabilities, supported object types | Version-aware compatibility checks and terminal capability profiles |
| Softkey layout | Physical key count, navigation-key count, key arrangement | Softkey canvas preview and terminal layout profiles |
| Languages | Supported/default languages, translation resources | Language resource management and variant generation |
| Authoring | ID allocation ranges, naming conventions | Configurable allocation and naming policies |
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
