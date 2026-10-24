//! Interactive preview and configuration for PictureGraphic conversion.
use super::{
    conversion::{
        self, ColourMapping, ConversionOptions, ConversionPreview, MappingMethod, SourceColour,
        TransparencyMode,
    },
    integration::GraphicService,
    preview, GraphicAssetId, PictureGraphicFormat,
};
use crate::{object_configuring, EditorProject};
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectType};
use eframe::egui;

#[derive(Clone)]
enum Source {
    Picture(u16),
    Import {
        target: u16,
        bytes: Vec<u8>,
    },
    Asset {
        id: GraphicAssetId,
        target: Option<u16>,
    },
    AssetOptions {
        id: GraphicAssetId,
        target: PictureGraphicFormat,
        request: egui::Id,
    },
}
#[derive(Clone)]
struct Draft {
    source: Source,
    session: uuid::Uuid,
    fingerprint: u64,
    options: ConversionOptions,
    filter: String,
    error: Option<String>,
    preview_cache: Option<(ConversionOptions, ConversionPreview)>,
}
fn draft_key(ctx: &egui::Context) -> egui::Id {
    ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("graphic_conversion_active")))
        .unwrap_or_else(|| egui::Id::new("graphic_conversion_draft"))
}
fn set_draft(ctx: &egui::Context, draft: Draft) {
    let key = egui::Id::new("graphic_conversion_draft");
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::new("graphic_conversion_active"), key);
        d.insert_temp(key, draft);
    });
}
fn fingerprint(project: &EditorProject, source: &Source) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match source {
        Source::Picture(id) => {
            id.hash(&mut h);
            if let Some(Object::PictureGraphic(p)) = ObjectId::new(*id)
                .ok()
                .and_then(|id| project.get_pool().object_by_id(id))
            {
                serde_json::to_vec(p).unwrap_or_default().hash(&mut h);
            }
        }
        Source::Import { target, bytes } => {
            target.hash(&mut h);
            hash_target(project, *target, &mut h);
            bytes.hash(&mut h);
        }
        Source::Asset { id, target } => {
            id.hash(&mut h);
            target.hash(&mut h);
            if let Some(target) = target {
                hash_target(project, *target, &mut h);
            }
            if let Some(a) = project.graphic_library().assets.get(id) {
                a.original_bytes.hash(&mut h);
            }
        }
        Source::AssetOptions {
            id,
            target,
            request,
        } => {
            id.hash(&mut h);
            u8::from(*target).hash(&mut h);
            request.hash(&mut h);
            project.revision().hash(&mut h);
            if let Some(a) = project.graphic_library().assets.get(id) {
                a.original_bytes.hash(&mut h);
            }
        }
    }
    for c in crate::effective_colour::colours(project.get_pool()) {
        c.as_rgb().hash(&mut h);
    }
    h.finish()
}
fn hash_target(project: &EditorProject, target: u16, hasher: &mut impl std::hash::Hasher) {
    if let Some(Object::PictureGraphic(picture)) = ObjectId::new(target)
        .ok()
        .and_then(|id| project.get_pool().object_by_id(id))
    {
        use std::hash::Hash;
        serde_json::to_vec(picture).unwrap_or_default().hash(hasher);
    }
}
fn target_format(
    project: &EditorProject,
    target: Option<u16>,
    requested: Option<PictureGraphicFormat>,
) -> PictureGraphicFormat {
    requested
        .or_else(|| {
            target
                .and_then(|v| ObjectId::new(v).ok())
                .and_then(|id| project.get_pool().object_by_id(id))
                .and_then(|o| match o {
                    Object::PictureGraphic(p) => Some(p.format),
                    _ => None,
                })
        })
        .unwrap_or(PictureGraphicFormat::EightBit)
}
fn draft(project: &EditorProject, source: Source, format: PictureGraphicFormat) -> Draft {
    let fingerprint = fingerprint(project, &source);
    Draft {
        source,
        session: project.ui_session_id(),
        fingerprint,
        options: suggested_options(project, format),
        filter: String::new(),
        error: None,
        preview_cache: None,
    }
}
fn suggested_options(project: &EditorProject, format: PictureGraphicFormat) -> ConversionOptions {
    let palette = crate::effective_colour::colours(project.get_pool()).map(|c| c.as_rgb());
    ConversionOptions::suggested_with_palette(format, &palette)
}
pub fn open_for_picture(
    ctx: &egui::Context,
    project: &EditorProject,
    id: ObjectId,
    requested: Option<PictureGraphicFormat>,
) {
    if let Some(Object::PictureGraphic(p)) = project.get_pool().object_by_id(id) {
        let mut d = draft(
            project,
            Source::Picture(id.value()),
            requested.unwrap_or(p.format),
        );
        d.options.transparency = if p.options.transparent {
            TransparencyMode::Preserve
        } else {
            TransparencyMode::Flatten
        };
        set_draft(ctx, d);
    }
}
pub fn open_for_import(
    ctx: &egui::Context,
    project: &EditorProject,
    target: ObjectId,
    bytes: Vec<u8>,
    requested: Option<PictureGraphicFormat>,
) {
    set_draft(
        ctx,
        draft(
            project,
            Source::Import {
                target: target.value(),
                bytes,
            },
            target_format(project, Some(target.value()), requested),
        ),
    );
}
pub fn open_for_asset(
    ctx: &egui::Context,
    project: &EditorProject,
    id: GraphicAssetId,
    target: Option<ObjectId>,
) {
    let target = target.map(|id| id.value());
    set_draft(
        ctx,
        draft(
            project,
            Source::Asset { id, target },
            target_format(project, target, None),
        ),
    );
}
/// Open a conversion that returns its options to a caller-owned atomic transaction.
pub fn open_asset_options(
    ctx: &egui::Context,
    project: &EditorProject,
    request: egui::Id,
    id: GraphicAssetId,
    target: PictureGraphicFormat,
) {
    set_draft(
        ctx,
        draft(
            project,
            Source::AssetOptions {
                id,
                target,
                request,
            },
            target,
        ),
    );
}
pub fn take_asset_options(ctx: &egui::Context, request: egui::Id) -> Option<ConversionOptions> {
    let key = egui::Id::new(("graphic_conversion_result", request));
    let result = ctx.data(|d| d.get_temp::<ConversionOptions>(key));
    ctx.data_mut(|d| d.remove::<ConversionOptions>(key));
    result
}
fn template(
    project: &EditorProject,
    target: Option<u16>,
) -> ag_iso_stack::object_pool::object::PictureGraphic {
    target
        .and_then(|id| ObjectId::new(id).ok())
        .and_then(|id| project.get_pool().object_by_id(id))
        .and_then(|o| match o {
            Object::PictureGraphic(p) => Some(p.clone()),
            _ => None,
        })
        .or_else(|| match crate::default_object(ObjectType::PictureGraphic) {
            Object::PictureGraphic(p) => Some(p),
            _ => None,
        })
        .unwrap()
}
fn preview_for(project: &EditorProject, d: &Draft) -> Result<ConversionPreview, String> {
    let palette = crate::effective_colour::colours(project.get_pool()).map(|c| c.as_rgb());
    match &d.source {
        Source::Picture(id) => {
            let p = ObjectId::new(*id)
                .ok()
                .and_then(|id| project.get_pool().object_by_id(id))
                .and_then(|o| match o {
                    Object::PictureGraphic(p) => Some(p),
                    _ => None,
                })
                .ok_or("PictureGraphic no longer exists")?;
            conversion::preview_picture(p, &d.options, &palette)
        }
        Source::Import { target, bytes } => conversion::preview_image(
            &template(project, Some(*target)),
            bytes,
            &d.options,
            &palette,
        ),
        Source::Asset { id, target } => {
            let a = project
                .graphic_library()
                .assets
                .get(id)
                .ok_or("Available graphic no longer exists")?;
            conversion::preview_image(
                &template(project, *target),
                &a.original_bytes,
                &d.options,
                &palette,
            )
        }
        Source::AssetOptions { id, .. } => {
            let a = project
                .graphic_library()
                .assets
                .get(id)
                .ok_or("Available graphic no longer exists")?;
            conversion::preview_image(
                &template(project, None),
                &a.original_bytes,
                &d.options,
                &palette,
            )
        }
    }
}
pub fn show(ctx: &egui::Context, project: &mut EditorProject) {
    let key = draft_key(ctx);
    let Some(mut d) = ctx.data(|data| data.get_temp::<Draft>(key)) else {
        return;
    };
    if d.session != project.ui_session_id() || d.fingerprint != fingerprint(project, &d.source) {
        ctx.data_mut(|data| {
            data.remove::<Draft>(key);
            data.remove::<egui::Id>(egui::Id::new("graphic_conversion_active"));
        });
        return;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        ctx.data_mut(|data| {
            data.remove::<Draft>(key);
            data.remove::<egui::Id>(egui::Id::new("graphic_conversion_active"));
        });
        return;
    }
    let computed = d
        .preview_cache
        .as_ref()
        .filter(|(options, _)| *options == d.options)
        .map(|(_, preview)| Ok(preview.clone()))
        .unwrap_or_else(|| preview_for(project, &d));
    let computed_options = d.options.clone();
    let preview_error = computed.as_ref().err();
    let displayed = computed
        .as_ref()
        .ok()
        .cloned()
        .or_else(|| d.preview_cache.as_ref().map(|(_, preview)| preview.clone()));
    let mut cancel = false;
    let mut apply = false;
    egui::Window::new("Convert PictureGraphic")
        .id(key.with("window"))
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .show(ctx, |ui| {
            let Some(p) = displayed.as_ref() else {
                if let Some(e) = preview_error.or(d.error.as_ref()) {
                    crate::ui_style::error_text(ui, e);
                }
                if ui.button("Reset to suggested").clicked() {
                    d.options = suggested_options(project, d.options.target);
                    d.error = None;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                return;
            };
            ui.horizontal(|ui| {
                ui.label("Target format:");
                for (f, n) in [
                    (PictureGraphicFormat::Monochrome, "Monochrome"),
                    (PictureGraphicFormat::FourBit, "4-bit"),
                    (PictureGraphicFormat::EightBit, "8-bit"),
                ] {
                    if ui.selectable_label(d.options.target == f, n).clicked() {
                        d.options = suggested_options(project, f);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Mapping:");
                egui::ComboBox::from_id_salt(key.with("mapping"))
                    .selected_text(if d.options.mapping == MappingMethod::Nearest {
                        "Nearest palette colour"
                    } else {
                        "Luminance threshold"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut d.options.mapping,
                            MappingMethod::Nearest,
                            "Nearest palette colour",
                        );
                        if d.options.target == PictureGraphicFormat::Monochrome {
                            ui.selectable_value(
                                &mut d.options.mapping,
                                MappingMethod::LuminanceThreshold,
                                "Luminance threshold",
                            );
                        }
                    });
                if d.options.mapping == MappingMethod::LuminanceThreshold {
                    ui.checkbox(&mut d.options.automatic_threshold, "Automatic threshold");
                    if !d.options.automatic_threshold {
                        ui.label("Threshold");
                        ui.add(egui::Slider::new(&mut d.options.threshold, 0..=255));
                    } else {
                        ui.label(format!("Suggested threshold: {}", p.threshold));
                    }
                    ui.checkbox(&mut d.options.invert, "Invert");
                    ui.label(format!("Computed threshold: {}", p.threshold));
                }
            });
            let palette = crate::effective_colour::colours(project.get_pool()).map(|c| c.as_rgb());
            ui.horizontal(|ui| {
                ui.label("Transparency:");
                ui.selectable_value(
                    &mut d.options.transparency,
                    TransparencyMode::Preserve,
                    "Preserve source",
                );
                ui.selectable_value(
                    &mut d.options.transparency,
                    TransparencyMode::Flatten,
                    "Flatten",
                );
                let range = 0..=match d.options.target {
                    PictureGraphicFormat::Monochrome => 1,
                    PictureGraphicFormat::FourBit => 15,
                    PictureGraphicFormat::EightBit => 255,
                };
                if d.options.transparency == TransparencyMode::Preserve {
                    let has_source_transparency = p
                        .colours
                        .iter()
                        .any(|colour| colour.suggested == ColourMapping::Transparent);
                    let has_manual_transparency = d
                        .options
                        .mappings
                        .values()
                        .any(|mapping| *mapping == ColourMapping::Transparent);
                    let needs_transparency = has_source_transparency || has_manual_transparency;
                    let mut reserve = needs_transparency || d.options.transparent_index.is_some();
                    if !needs_transparency
                        && ui
                            .checkbox(&mut reserve, "Reserve transparent index")
                            .changed()
                    {
                        d.options.transparent_index = reserve.then_some(
                            d.options
                                .transparent_index
                                .unwrap_or(p.picture.transparency_colour),
                        );
                    }
                    if reserve {
                        ui.label("Transparent output index");
                        let index = d
                            .options
                            .transparent_index
                            .unwrap_or(p.picture.transparency_colour);
                        if let Some(i) =
                            object_configuring::colour_index_picker(ui, index, &palette, range)
                        {
                            d.options.transparent_index = Some(i);
                        }
                    }
                } else {
                    ui.label("Background");
                    if let Some(i) = object_configuring::colour_index_picker(
                        ui,
                        d.options.flatten_background,
                        &palette,
                        range,
                    ) {
                        d.options.flatten_background = i;
                    }
                }
            });
            if !matches!(d.source, Source::Picture(_))
                && d.options.transparency == TransparencyMode::Preserve
            {
                ui.horizontal(|ui| {
                    ui.label("Alpha cutoff");
                    ui.add(egui::Slider::new(&mut d.options.alpha_cutoff, 0..=255));
                });
            }
            if d.options.target == PictureGraphicFormat::Monochrome {
                ui.horizontal(|ui| {
                    ui.label("Preset:");
                    if ui.button("Black opaque / white transparent").clicked() {
                        monochrome_preset(&mut d.options, p, &palette, false, true);
                    }
                    if ui.button("White opaque / black transparent").clicked() {
                        monochrome_preset(&mut d.options, p, &palette, true, true);
                    }
                    if ui.button("Both opaque").clicked() {
                        monochrome_preset(&mut d.options, p, &palette, false, false);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Source");
                    if let Some(t) = source_texture(ctx, project, &d.source) {
                        draw_preview(ui, &t);
                    }
                });
                ui.vertical(|ui| {
                    ui.label("Result");
                    let img = result_image(&p.picture, project);
                    let t = ctx.load_texture(
                        format!("conversion_result_{key:?}"),
                        img,
                        Default::default(),
                    );
                    draw_preview(ui, &t);
                });
            });
            let output_indices = super::indexed_png::pixel_indices(&p.picture).unwrap_or_default();
            let transparent_pixels = if p.picture.options.transparent {
                output_indices
                    .iter()
                    .filter(|i| **i == p.picture.transparency_colour)
                    .count()
            } else {
                0
            };
            let result_colours = output_indices
                .iter()
                .copied()
                .filter(|i| !p.picture.options.transparent || *i != p.picture.transparency_colour)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            ui.label(format!(
                "{} × {} · {} source colours → {} result colours · {} transparent pixels",
                p.picture.actual_width,
                p.picture.actual_height,
                p.colours.len(),
                result_colours,
                transparent_pixels
            ));
            ui.label(egui::RichText::new(p.data_summary()).weak())
                .on_hover_text(p.data_tooltip());
            ui.horizontal(|ui| {
                ui.label("Colour mappings");
                ui.add(egui::TextEdit::singleline(&mut d.filter).hint_text("Search colours…"));
            });
            let filtered: Vec<_> = p
                .colours
                .iter()
                .filter(|row| {
                    source_label(row.source, &palette)
                        .to_lowercase()
                        .contains(&d.filter.to_lowercase())
                })
                .collect();
            egui::ScrollArea::vertical().max_height(200.0).show_rows(
                ui,
                28.0,
                filtered.len(),
                |ui, rows| {
                    for r in rows {
                        let row = filtered[r];
                        let label = source_label(row.source, &palette);
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.label(format!("{} px", row.pixels));
                            ui.label("→");
                            let mapping = d
                                .options
                                .mappings
                                .get(&row.source)
                                .copied()
                                .unwrap_or(row.suggested);
                            let picker_index = match mapping {
                                ColourMapping::Index(index) => index,
                                ColourMapping::Transparent => {
                                    ui.label("Transparent");
                                    if p.picture.transparency_colour == 0 {
                                        1
                                    } else {
                                        0
                                    }
                                }
                            };
                            let range = 0..=match d.options.target {
                                PictureGraphicFormat::Monochrome => 1,
                                PictureGraphicFormat::FourBit => 15,
                                PictureGraphicFormat::EightBit => 255,
                            };
                            if let Some(index) = object_configuring::colour_index_picker(
                                ui,
                                picker_index,
                                &palette,
                                range,
                            ) {
                                d.options
                                    .mappings
                                    .insert(row.source, ColourMapping::Index(index));
                            }
                            let suggestion = match row.suggested {
                                ColourMapping::Index(index) => {
                                    let [r, g, b] = palette[usize::from(index)];
                                    format!("Suggested: {index} · #{r:02X}{g:02X}{b:02X}")
                                }
                                ColourMapping::Transparent => "Suggested: transparent".to_owned(),
                            };
                            ui.label(egui::RichText::new(suggestion).weak());
                            if d.options.transparency == TransparencyMode::Preserve
                                && ui.small_button("Transparent").clicked()
                            {
                                d.options
                                    .mappings
                                    .insert(row.source, ColourMapping::Transparent);
                            }
                            if ui.small_button("Auto").clicked() {
                                d.options.mappings.remove(&row.source);
                            }
                        });
                    }
                },
            );
            if let Some(e) = preview_error.or(d.error.as_ref()) {
                crate::ui_style::error_text(ui, e);
            }
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                if ui.button("Reset to suggested").clicked() {
                    d.options = suggested_options(project, d.options.target);
                    d.filter.clear();
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if apply {
        if preview_for(project, &d).is_ok() {
            let result = match &d.source {
                Source::Picture(id) => GraphicService::convert_picture(project, *id, &d.options),
                Source::Import { target, bytes } => {
                    GraphicService::convert_image_into_picture_graphic(
                        project, *target, bytes, &d.options,
                    )
                }
                Source::Asset {
                    id,
                    target: Some(target),
                } => GraphicService::convert_asset_into_picture_graphic_with_options(
                    project, *id, *target, &d.options,
                ),
                Source::Asset { id, target: None } => {
                    GraphicService::create_picture_graphic_from_asset_with_options(
                        project, *id, &d.options,
                    )
                    .map(|_| ())
                }
                Source::AssetOptions { request, .. } => {
                    ctx.data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new(("graphic_conversion_result", *request)),
                            d.options.clone(),
                        )
                    });
                    Ok(())
                }
            };
            if let Err(e) = result {
                d.error = Some(e.to_string());
            } else {
                cancel = true;
            }
        }
    }
    if cancel {
        ctx.data_mut(|data| {
            data.remove::<Draft>(key);
            data.remove::<egui::Id>(egui::Id::new("graphic_conversion_active"));
        });
    } else {
        if d.options == computed_options {
            if let Ok(p) = computed {
                d.preview_cache = Some((d.options.clone(), p));
            }
        } else if let Ok(p) = preview_for(project, &d) {
            d.preview_cache = Some((d.options.clone(), p));
        }
        ctx.data_mut(|data| data.insert_temp(key, d));
    }
}
fn draw_preview(ui: &mut egui::Ui, texture: &egui::TextureHandle) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 180.0), egui::Sense::hover());
    crate::object_picker::checkerboard(ui.painter(), rect);
    crate::object_picker::texture(ui, rect, texture);
}
fn source_texture(
    ctx: &egui::Context,
    project: &EditorProject,
    source: &Source,
) -> Option<egui::TextureHandle> {
    match source {
        Source::Picture(id) => ObjectId::new(*id)
            .ok()
            .and_then(|id| project.get_pool().object_by_id(id))
            .and_then(|o| match o {
                Object::PictureGraphic(p) => preview::picture_texture(ctx, p, project.get_pool()),
                _ => None,
            }),
        Source::Import { bytes, .. } => image::load_from_memory(bytes).ok().map(|img| {
            ctx.load_texture(
                "conversion_source",
                egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.to_rgba8().as_raw(),
                ),
                Default::default(),
            )
        }),
        Source::Asset { id, .. } | Source::AssetOptions { id, .. } => project
            .graphic_library()
            .assets
            .get(id)
            .and_then(|a| preview::asset_texture(ctx, a)),
    }
}
fn result_image(
    p: &ag_iso_stack::object_pool::object::PictureGraphic,
    project: &EditorProject,
) -> egui::ColorImage {
    let palette = crate::effective_colour::colours(project.get_pool()).map(|c| c.as_rgb());
    let indices = super::indexed_png::pixel_indices(p).unwrap_or_default();
    let pixels = indices
        .into_iter()
        .map(|i| {
            if p.options.transparent && i == p.transparency_colour {
                egui::Color32::TRANSPARENT
            } else {
                let [r, g, b] = palette[usize::from(i)];
                egui::Color32::from_rgb(r, g, b)
            }
        })
        .collect();
    egui::ColorImage::new(
        [usize::from(p.actual_width), usize::from(p.actual_height)],
        pixels,
    )
}
fn source_label(source: SourceColour, palette: &[[u8; 3]; 256]) -> String {
    match source {
        SourceColour::Index(i) => {
            let [r, g, b] = palette[usize::from(i)];
            format!("Index {i} · #{r:02X}{g:02X}{b:02X}")
        }
        SourceColour::Rgba([r, g, b, a]) => format!("#{r:02X}{g:02X}{b:02X}{a:02X}"),
    }
}
fn monochrome_preset(
    options: &mut ConversionOptions,
    p: &ConversionPreview,
    palette: &[[u8; 3]; 256],
    invert: bool,
    transparent: bool,
) {
    options.mapping = MappingMethod::Nearest;
    options.transparency = if transparent {
        TransparencyMode::Preserve
    } else {
        TransparencyMode::Flatten
    };
    let nearest = |target: [u8; 3]| {
        (0..=1u8)
            .min_by_key(|index| {
                palette[usize::from(*index)]
                    .into_iter()
                    .zip(target)
                    .map(|(a, b)| (i32::from(a) - i32::from(b)).pow(2) as u32)
                    .sum::<u32>()
            })
            .unwrap()
    };
    let black = nearest([0, 0, 0]);
    let white = nearest([255, 255, 255]);
    options.transparent_index = transparent.then_some(if invert { black } else { white });
    options.mappings.clear();
    for row in &p.colours {
        let rgb = match row.source {
            SourceColour::Index(i) => palette[usize::from(i)],
            SourceColour::Rgba([r, g, b, _]) => [r, g, b],
        };
        let light =
            299 * u32::from(rgb[0]) + 587 * u32::from(rgb[1]) + 114 * u32::from(rgb[2]) >= 128_000;
        let value = if transparent && ((light && !invert) || (!light && invert)) {
            ColourMapping::Transparent
        } else if light {
            ColourMapping::Index(white)
        } else {
            ColourMapping::Index(black)
        };
        options.mappings.insert(row.source, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{ObjectPool, ObjectType};

    fn fixture() -> (EditorProject, ObjectId) {
        let id = ObjectId::new(42).unwrap();
        let mut object = crate::default_object(ObjectType::PictureGraphic);
        object.mut_id().set_value(id.value()).unwrap();
        let Object::PictureGraphic(picture) = &mut object else {
            unreachable!()
        };
        picture.width = 1;
        picture.actual_width = 1;
        picture.actual_height = 1;
        picture.data = vec![1];
        let mut pool = ObjectPool::new();
        pool.add(object);
        (EditorProject::from(pool), id)
    }

    #[test]
    fn canceling_the_headless_dialog_preserves_the_picture() {
        let (mut project, id) = fixture();
        let before = project.get_pool().object_by_id(id).unwrap().clone();
        let ctx = egui::Context::default();
        open_for_picture(&ctx, &project, id, Some(PictureGraphicFormat::Monochrome));
        let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &mut project));
        assert!(ctx
            .data(|data| data.get_temp::<Draft>(draft_key(&ctx)))
            .is_some());

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        let _ = ctx.run(input, |ctx| show(ctx, &mut project));
        assert!(ctx
            .data(|data| data.get_temp::<Draft>(draft_key(&ctx)))
            .is_none());
        assert_eq!(project.get_pool().object_by_id(id).unwrap(), &before);
    }

    #[test]
    fn changed_target_invalidates_the_conversion_draft() {
        let (mut project, id) = fixture();
        let ctx = egui::Context::default();
        open_for_import(&ctx, &project, id, vec![1, 2, 3], None);
        let options = ConversionOptions::suggested(PictureGraphicFormat::FourBit);
        GraphicService::convert_picture(&mut project, id.value(), &options).unwrap();
        let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &mut project));
        assert!(ctx
            .data(|data| data.get_temp::<Draft>(draft_key(&ctx)))
            .is_none());
    }

    #[test]
    fn options_only_result_round_trips_through_egui_memory() {
        let ctx = egui::Context::default();
        let request = egui::Id::new("conversion-options-request");
        let options = ConversionOptions::suggested(PictureGraphicFormat::Monochrome);
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new(("graphic_conversion_result", request)),
                options.clone(),
            );
        });
        assert_eq!(take_asset_options(&ctx, request), Some(options));
        assert_eq!(take_asset_options(&ctx, request), None);
    }
}
