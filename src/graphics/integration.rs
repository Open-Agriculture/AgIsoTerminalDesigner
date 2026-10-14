//! Project integration for the two-state graphics model.
use super::*;
use crate::{components::ComponentError, EditorProject};
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool, ObjectType};
use std::collections::HashMap;

/// Assets are converted before materialization, so rendering only sees ordinary
/// PictureGraphic objects. Kept as a compatibility boundary for materializers.
pub fn apply(
    _source: &crate::components::ComponentSource,
    _graphics: &GraphicLibrary,
    _pool: &mut ObjectPool,
) -> crate::components::ComponentResult<()> {
    Ok(())
}

pub struct GraphicService;

pub(crate) fn consume_into_picture_graphic(
    pool: &mut ObjectPool,
    graphics: &mut GraphicLibrary,
    asset_id: GraphicAssetId,
    object_id: u16,
) -> crate::components::ComponentResult<GraphicAsset> {
    let asset = graphics
        .assets
        .get(&asset_id)
        .cloned()
        .ok_or_else(|| ComponentError::new("GRAPHIC_ASSET_NOT_FOUND", "Unknown graphic"))?;
    let picture = ObjectId::new(object_id)
        .ok()
        .and_then(|id| pool.object_mut_by_id(id));
    let Some(Object::PictureGraphic(picture)) = picture else {
        return Err(ComponentError::new(
            "GRAPHIC_TARGET_INVALID",
            format!("Target {object_id} must be a PictureGraphic"),
        ));
    };
    apply_image_bytes_to_picture_graphic(picture, &asset.original_bytes)
        .map_err(|error| ComponentError::new("GRAPHIC_CONVERSION_FAILED", error))?;
    graphics.assets.remove(&asset_id);
    Ok(asset)
}

/// Recover the current graphic; consumed uploads intentionally retain no source binding.
pub(crate) fn recover_picture_graphic(
    picture: &ag_iso_stack::object_pool::object::PictureGraphic,
    pool: &ObjectPool,
    info: Option<&crate::ObjectInfo>,
) -> Result<GraphicAsset, String> {
    let object = Object::PictureGraphic(picture.clone());
    let name = info
        .map(|info| info.get_name(&object))
        .unwrap_or_else(|| format!("PictureGraphic {}", picture.id.value()));
    let bytes = super::preview::picture_png(picture, pool)?;
    let mut asset =
        GraphicAsset::from_upload(&format!("PictureGraphic-{}.png", picture.id.value()), bytes)?;
    asset.name = name;
    asset.description = info.and_then(|info| info.description.clone());
    asset.tags = info.map(|info| info.tags.clone()).unwrap_or_default();
    Ok(asset)
}

impl GraphicService {
    pub fn upload(
        project: &mut EditorProject,
        filename: &str,
        bytes: Vec<u8>,
    ) -> crate::components::ComponentResult<GraphicAssetId> {
        let asset = GraphicAsset::from_upload(filename, bytes)
            .map_err(|error| ComponentError::new("GRAPHIC_CONVERSION_FAILED", error))?;
        let id = asset.id;
        project.edit_source("Upload graphic", |_, _, graphics, _| {
            graphics.assets.insert(id, asset);
            Ok(())
        })?;
        Ok(id)
    }

    pub fn rename_available_asset(
        project: &mut EditorProject,
        id: GraphicAssetId,
        name: String,
    ) -> crate::components::ComponentResult<()> {
        project.edit_source("Rename available graphic", |_, _, graphics, _| {
            graphics
                .assets
                .get_mut(&id)
                .ok_or_else(|| ComponentError::new("GRAPHIC_ASSET_NOT_FOUND", "Unknown graphic"))?
                .name = name;
            Ok(())
        })
    }

    pub fn delete_available_asset(
        project: &mut EditorProject,
        id: GraphicAssetId,
    ) -> crate::components::ComponentResult<()> {
        project.edit_source("Delete available graphic", |_, _, graphics, _| {
            graphics
                .assets
                .remove(&id)
                .ok_or_else(|| ComponentError::new("GRAPHIC_ASSET_NOT_FOUND", "Unknown graphic"))?;
            Ok(())
        })
    }

    pub fn convert_asset_into_picture_graphic(
        project: &mut EditorProject,
        asset_id: GraphicAssetId,
        object_id: u16,
    ) -> crate::components::ComponentResult<()> {
        project.edit_source(
            "Convert graphic into PictureGraphic",
            |pool, _, graphics, info| {
                let asset = consume_into_picture_graphic(pool, graphics, asset_id, object_id)?;
                let object = pool
                    .object_by_id(ObjectId::new(object_id).unwrap())
                    .unwrap();
                let entry = info
                    .entry(object.id())
                    .or_insert_with(|| crate::ObjectInfo::new(object));
                entry.name = Some(asset.name);
                entry.description = asset.description;
                entry.tags = asset.tags;
                Ok(())
            },
        )
    }

    pub fn create_picture_graphic_from_asset(
        project: &mut EditorProject,
        asset_id: GraphicAssetId,
    ) -> crate::components::ComponentResult<u16> {
        let mut created_id = None;
        let ranges = project.object_id_ranges().clone();
        project.edit_source(
            "Create PictureGraphic from graphic",
            |pool, source, graphics, info| {
                let asset = graphics.assets.get(&asset_id).cloned().ok_or_else(|| {
                    ComponentError::new("GRAPHIC_ASSET_NOT_FOUND", "Unknown graphic")
                })?;
                let range = ranges.range_for(ObjectType::PictureGraphic);
                let id = (range.start..=range.end)
                    .find(|id| {
                        !pool
                            .objects()
                            .iter()
                            .any(|object| object.id().value() == *id)
                            && !source.instances.iter().any(|instance| {
                                instance
                                    .compiled_ids
                                    .values()
                                    .any(|reserved| reserved == id)
                            })
                    })
                    .ok_or_else(|| {
                        ComponentError::new(
                            "OBJECT_ID_RANGE_EXHAUSTED",
                            format!(
                                "No free PictureGraphic ID remains in configured range {range}"
                            ),
                        )
                    })?;
                let mut object = crate::default_object(ObjectType::PictureGraphic);
                object.mut_id().set_value(id).map_err(|error| {
                    ComponentError::new("OBJECT_ID_INVALID", format!("{error:?}"))
                })?;
                let Object::PictureGraphic(picture) = &mut object else {
                    unreachable!("PictureGraphic defaults must have PictureGraphic type")
                };
                apply_image_bytes_to_picture_graphic(picture, &asset.original_bytes)
                    .map_err(|error| ComponentError::new("GRAPHIC_CONVERSION_FAILED", error))?;
                let object_id = object.id();
                pool.add(object);
                graphics.assets.remove(&asset_id);
                let object = pool.object_by_id(object_id).unwrap();
                let entry = info
                    .entry(object_id)
                    .or_insert_with(|| crate::ObjectInfo::new(object));
                entry.name = Some(asset.name);
                entry.description = asset.description;
                entry.tags = asset.tags;
                created_id = Some(id);
                Ok(())
            },
        )?;
        Ok(created_id.expect("successful conversion creates an object ID"))
    }
}

pub fn catalog(
    pool: &ObjectPool,
    graphics: &GraphicLibrary,
    info: &HashMap<ObjectId, crate::ObjectInfo>,
    query: &str,
    status: &str,
    limit: usize,
) -> serde_json::Value {
    let query = query.to_lowercase();
    let mut entries = Vec::new();
    if !status.eq_ignore_ascii_case("available") {
        for object in pool.objects_by_type(ObjectType::PictureGraphic) {
            let metadata = info.get(&object.id());
            let name = metadata
                .and_then(|metadata| metadata.name.clone())
                .unwrap_or_else(|| format!("PictureGraphic {}", object.id().value()));
            let description = metadata.and_then(|metadata| metadata.description.clone());
            let tags = metadata
                .map(|metadata| metadata.tags.clone())
                .unwrap_or_default();
            let text = format!(
                "{} {} {} {}",
                object.id().value(),
                name,
                description.as_deref().unwrap_or(""),
                tags.join(" "),
            )
            .to_lowercase();
            if text.contains(&query) {
                entries.push(serde_json::json!({
                    "kind": "picture_graphic",
                    "object_id": object.id().value(),
                    "name": name,
                    "description": description,
                    "tags": tags,
                }));
            }
        }
    }
    if !status.eq_ignore_ascii_case("picture_graphic") {
        for asset in graphics.assets.values() {
            let text = format!(
                "{} {} {} {}",
                asset.name,
                asset.description.as_deref().unwrap_or(""),
                asset.tags.join(" "),
                asset.original_file_name.as_deref().unwrap_or(""),
            )
            .to_lowercase();
            if text.contains(&query) {
                entries.push(serde_json::json!({
                    "kind": "available_asset",
                    "asset_id": asset.id,
                    "name": asset.name,
                    "description": asset.description,
                    "tags": asset.tags,
                }));
            }
        }
    }
    serde_json::json!({
        "total": entries.len(),
        "graphics": entries.into_iter().take(limit.clamp(1, 50)).collect::<Vec<_>>(),
    })
}
