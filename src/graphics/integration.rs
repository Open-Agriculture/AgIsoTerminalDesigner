//! Project integration: one resolver shared by materialization, UI and AI.
use super::*;
use crate::{components::*, EditorProject};
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};

pub fn assignments(
    source: &ComponentSource,
    graphics: &GraphicLibrary,
) -> ComponentResult<BTreeMap<GraphicAssetId, PictureGraphicSource>> {
    let mut owners = BTreeMap::new();
    let mut insert = |asset: GraphicAssetId, target: PictureGraphicSource| -> ComponentResult<()> {
        if !graphics.assets.contains_key(&asset) {
            return Err(ComponentError::new(
                "GRAPHIC_ASSET_NOT_FOUND",
                format!("Unknown graphic {asset}"),
            ));
        }
        if let Some(existing) = owners.insert(asset, target.clone()) {
            return Err(ComponentError::new("GRAPHIC_ALREADY_ASSIGNED", format!("asset_id={asset}; existing PictureGraphic={existing:?}; requested target={target:?}. Reuse the existing PictureGraphic through an object reference or child link; this node cannot own it again.")));
        }
        Ok(())
    };
    for (object_id, binding) in &graphics.picture_graphic_bindings {
        insert(
            binding.asset_id,
            PictureGraphicSource::Primitive {
                object_id: *object_id,
            },
        )?;
    }
    // Defaults in unused definitions do not own assets. Each actual instance does.
    for instance in &source.instances {
        let definition = source
            .definitions
            .get(&instance.component_id)
            .ok_or_else(|| ComponentError::new("COMPONENT_NOT_FOUND", "Unknown component"))?;
        for parameter in &definition.parameters {
            if parameter.kind == ComponentParameterKind::Graphic {
                let value = instance
                    .overrides
                    .get(&parameter.id)
                    .unwrap_or(&parameter.default_value);
                let asset = serde_json::from_value(value.clone()).map_err(|_| {
                    ComponentError::new(
                        "COMPONENT_OVERRIDE_TYPE_MISMATCH",
                        "Graphic values must be asset UUIDs",
                    )
                })?;
                insert(
                    asset,
                    PictureGraphicSource::ComponentNode {
                        instance_id: instance.id.0,
                        node_id: parameter.target_node.0,
                    },
                )?;
            }
        }
    }
    Ok(owners)
}
pub fn apply(
    source: &ComponentSource,
    graphics: &GraphicLibrary,
    pool: &mut ObjectPool,
) -> ComponentResult<()> {
    for (key, asset) in &graphics.assets {
        if *key != asset.id {
            return Err(ComponentError::new(
                "GRAPHIC_ASSET_INVALID",
                "Asset key does not match identity",
            ));
        }
    }
    for (asset, owner) in assignments(source, graphics)? {
        let id = object_id(source, &owner).ok_or_else(|| {
            ComponentError::new(
                "GRAPHIC_TARGET_INVALID",
                "PictureGraphic source no longer exists",
            )
        })?;
        let object = ObjectId::new(id)
            .ok()
            .and_then(|id| pool.object_mut_by_id(id));
        let Some(Object::PictureGraphic(picture)) = object else {
            return Err(ComponentError::new(
                "GRAPHIC_TARGET_INVALID",
                format!("Target {id} must be a PictureGraphic"),
            ));
        };
        apply_image_bytes_to_picture_graphic(picture, &graphics.assets[&asset].original_bytes)
            .map_err(|e| ComponentError::new("GRAPHIC_CONVERSION_FAILED", e))?;
        if let PictureGraphicSource::Primitive { object_id } = owner {
            let binding = &graphics.picture_graphic_bindings[&object_id];
            if let Some(value) = binding.transparent {
                picture.options.transparent = value;
            }
            if let Some(value) = binding.transparency_colour {
                picture.transparency_colour = value;
            }
            if let Some(value) = binding.flashing {
                picture.options.flashing = value;
            }
            if let Some(format) = binding.format {
                if picture.format != format {
                    convert_picture_graphic_format(picture, format);
                }
            }
        }
    }
    Ok(())
}
pub fn object_id(source: &ComponentSource, owner: &PictureGraphicSource) -> Option<u16> {
    match owner {
        PictureGraphicSource::Primitive { object_id } => Some(*object_id),
        PictureGraphicSource::ComponentNode {
            instance_id,
            node_id,
        } => source
            .instances
            .iter()
            .find(|i| i.id.0 == *instance_id)?
            .compiled_ids
            .get(&ComponentNodeId(*node_id))
            .copied(),
    }
}
pub struct GraphicService;
impl GraphicService {
    /// Resolve an assigned asset to the ordinary reference model. Generated nodes
    /// remain encapsulated by the component model and cannot be linked externally.
    pub fn reuse_target(
        project: &EditorProject,
        asset: GraphicAssetId,
    ) -> ComponentResult<crate::ObjectTarget> {
        match project.picture_for_graphic(asset)? {
            Some(PictureGraphicSource::Primitive { object_id }) => Ok(crate::ObjectTarget::id(object_id)),
            Some(owner) => Err(ComponentError::new("GRAPHIC_REUSE_NOT_SUPPORTED", format!("{owner:?} is encapsulated by its component; reuse within that component or detach it first"))),
            None => Err(ComponentError::new("GRAPHIC_NOT_ASSIGNED", "Assign this available graphic to a PictureGraphic first")),
        }
    }
    pub fn reuse_child(
        project: &mut EditorProject,
        asset: GraphicAssetId,
        parent: u16,
    ) -> ComponentResult<()> {
        let child = Self::reuse_target(project, asset)?;
        let transaction = crate::OperationTransaction {
            schema_version: crate::CURRENT_OPERATION_SCHEMA_VERSION,
            description: Some("Reuse existing PictureGraphic".into()),
            operations: vec![crate::Operation::AddChild {
                parent: crate::ObjectTarget::id(parent),
                child,
                x: 0,
                y: 0,
            }],
        };
        project
            .execute_transaction(transaction)
            .map(|_| ())
            .map_err(|e| ComponentError::new("GRAPHIC_REUSE_NOT_SUPPORTED", e.to_string()))
    }

    pub fn upload(
        project: &mut EditorProject,
        filename: &str,
        bytes: Vec<u8>,
    ) -> ComponentResult<GraphicAssetId> {
        let asset = GraphicAsset::from_upload(filename, bytes)
            .map_err(|e| ComponentError::new("GRAPHIC_CONVERSION_FAILED", e))?;
        let id = asset.id;
        project.edit_source("Upload graphic", |_, _, graphics| {
            graphics.assets.insert(id, asset);
            Ok(())
        })?;
        Ok(id)
    }
    pub fn rename(
        project: &mut EditorProject,
        id: GraphicAssetId,
        name: String,
    ) -> ComponentResult<()> {
        project.edit_source("Rename graphic", |_, _, graphics| {
            graphics
                .assets
                .get_mut(&id)
                .ok_or_else(|| ComponentError::new("GRAPHIC_ASSET_NOT_FOUND", "Unknown graphic"))?
                .name = name;
            Ok(())
        })
    }
    pub fn delete(project: &mut EditorProject, id: GraphicAssetId) -> ComponentResult<()> {
        project.edit_source("Delete graphic", |_, source, graphics| {
            if let Some(owner) = assignments(source, graphics)?.get(&id) {
                return Err(ComponentError::new(
                    "GRAPHIC_ASSET_ASSIGNED",
                    format!("Graphic {id} is assigned to {owner:?}"),
                ));
            }
            graphics
                .assets
                .remove(&id)
                .ok_or_else(|| ComponentError::new("GRAPHIC_ASSET_NOT_FOUND", "Unknown graphic"))?;
            Ok(())
        })
    }
    pub fn set_options(
        project: &mut EditorProject,
        object_id: u16,
        transparent: bool,
        colour: u8,
        flashing: bool,
    ) -> ComponentResult<()> {
        project.edit_source("Edit assigned graphic options", |_, _, graphics| {
            let binding = graphics
                .picture_graphic_bindings
                .get_mut(&object_id)
                .ok_or_else(|| {
                    ComponentError::new(
                        "GRAPHIC_NOT_ASSIGNED",
                        "PictureGraphic has no asset binding",
                    )
                })?;
            binding.transparent = Some(transparent);
            binding.transparency_colour = Some(colour);
            binding.flashing = Some(flashing);
            Ok(())
        })
    }
    pub fn set_format(
        project: &mut EditorProject,
        object_id: u16,
        format: PictureGraphicFormat,
    ) -> ComponentResult<()> {
        project.edit_source("Convert assigned graphic format", |_, _, graphics| {
            graphics
                .picture_graphic_bindings
                .get_mut(&object_id)
                .ok_or_else(|| {
                    ComponentError::new(
                        "GRAPHIC_NOT_ASSIGNED",
                        "PictureGraphic has no asset binding",
                    )
                })?
                .format = Some(format);
            Ok(())
        })
    }
    pub fn assign(
        project: &mut EditorProject,
        object_id: u16,
        asset: Option<GraphicAssetId>,
    ) -> ComponentResult<()> {
        project.edit_source("Assign graphic", |_, _, graphics| {
            graphics.set_binding(object_id, asset);
            Ok(())
        })
    }
}
impl EditorProject {
    pub fn picture_for_graphic(
        &self,
        asset: GraphicAssetId,
    ) -> ComponentResult<Option<PictureGraphicSource>> {
        Ok(assignments(self.component_source(), self.graphic_library())?.remove(&asset))
    }
    pub fn graphic_for_picture(
        &self,
        target: &PictureGraphicSource,
    ) -> ComponentResult<Option<GraphicAssetId>> {
        Ok(
            assignments(self.component_source(), self.graphic_library())?
                .into_iter()
                .find(|(_, owner)| owner == target)
                .map(|(asset, _)| asset),
        )
    }
}
pub fn catalog(
    source: &ComponentSource,
    graphics: &GraphicLibrary,
    query: &str,
    status: &str,
    limit: usize,
) -> ComponentResult<serde_json::Value> {
    let owners = assignments(source, graphics)?;
    let mut entries = vec![];
    for asset in graphics.assets.values() {
        let owner = owners.get(&asset.id);
        let state = if owner.is_some() {
            "assigned"
        } else {
            "available"
        };
        let text = format!(
            "{} {} {}",
            asset.name,
            asset.description.as_deref().unwrap_or(""),
            asset.tags.join(" ")
        )
        .to_lowercase();
        if !text.contains(&query.to_lowercase())
            || (!status.eq_ignore_ascii_case("all") && !status.eq_ignore_ascii_case(state))
        {
            continue;
        }
        entries.push(serde_json::json!({"id":asset.id,"name":asset.name,"description":asset.description,"tags":asset.tags,"status":state,"picture_graphic":owner.map(|o| serde_json::json!({"source":o,"object_id":object_id(source,o)}))}));
    }
    Ok(
        serde_json::json!({"total":entries.len(),"assets":entries.into_iter().take(limit.clamp(1,50)).collect::<Vec<_>>()}),
    )
}
