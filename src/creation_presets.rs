//! Starting values for objects created from the editor.
//!
//! The ISO defaults in `object_defaults` describe a 0 x 0 object with
//! attribute references to object 0, which is invisible and invalid on a mask.
//! Objects created in the editor start from these presets instead, so they can
//! be seen and moved as soon as they are placed.

use std::collections::HashMap;

use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool, ObjectType};
use serde_json::{json, Value};

use crate::object_properties::{get_object_property, PropertyKey};

/// A property a new object starts with instead of its ISO default.
pub(crate) enum Preset {
    Value(PropertyKey, Value),
    /// A required attribute reference, filled with an attribute object of this type.
    Attribute(PropertyKey, ObjectType),
}

pub(crate) fn presets(object_type: ObjectType) -> Vec<Preset> {
    use Preset::{Attribute, Value as Set};
    use PropertyKey as P;

    let size =
        |width: u16, height: u16| vec![Set(P::Width, json!(width)), Set(P::Height, json!(height))];
    let font = || Attribute(P::FontAttributes, ObjectType::FontAttributes);
    let line = || Attribute(P::LineAttributes, ObjectType::LineAttributes);
    let with = |mut presets: Vec<Preset>, more: Vec<Preset>| {
        presets.extend(more);
        presets
    };

    match object_type {
        ObjectType::Container | ObjectType::Animation | ObjectType::ScaledGraphic => size(100, 100),
        ObjectType::Button => with(size(100, 50), vec![Set(P::BackgroundColour, json!(7))]),
        ObjectType::InputBoolean => vec![
            Set(P::Width, json!(24)),
            Attribute(P::ForegroundColour, ObjectType::FontAttributes),
        ],
        ObjectType::InputString | ObjectType::OutputString => with(size(100, 24), vec![font()]),
        ObjectType::InputNumber | ObjectType::OutputNumber => with(size(80, 24), vec![font()]),
        ObjectType::InputList | ObjectType::OutputList => size(100, 24),
        ObjectType::OutputLine | ObjectType::OutputRectangle => with(size(100, 50), vec![line()]),
        ObjectType::OutputEllipse => with(size(50, 50), vec![line()]),
        ObjectType::OutputPolygon => with(
            size(100, 100),
            vec![
                Set(
                    P::Points,
                    json!([{ "x": 0, "y": 99 }, { "x": 50, "y": 0 }, { "x": 99, "y": 99 }]),
                ),
                line(),
            ],
        ),
        // Half a circle, over the top
        ObjectType::OutputMeter => vec![
            Set(P::Width, json!(100)),
            Set(P::EndAngle, json!(90)),
            Set(P::MaxValue, json!(100)),
        ],
        ObjectType::OutputLinearBarGraph => with(size(100, 24), vec![Set(P::MaxValue, json!(100))]),
        ObjectType::OutputArchedBarGraph => with(
            size(100, 100),
            vec![
                Set(P::EndAngle, json!(90)),
                Set(P::BarGraphWidth, json!(10)),
                Set(P::MaxValue, json!(100)),
            ],
        ),
        // A solid line one pixel wide; the ISO default draws nothing
        ObjectType::LineAttributes => {
            vec![Set(P::LineWidth, json!(1)), Set(P::LineArt, json!(0xFFFF))]
        }
        _ => vec![],
    }
}

/// The attribute object of the given type that the pool references most,
/// preferring the lowest ID on a tie.
pub(crate) fn most_used_attribute(
    pool: &ObjectPool,
    attribute_type: ObjectType,
) -> Option<ObjectId> {
    let mut uses: HashMap<u16, usize> = pool
        .objects_by_type(attribute_type)
        .iter()
        .map(|object| (object.id().value(), 0))
        .collect();
    for object in pool.objects() {
        // Foreground colour is a Font Attributes reference only on Input Boolean
        let keys: &[PropertyKey] = if matches!(object, Object::InputBoolean(_)) {
            &[PropertyKey::ForegroundColour]
        } else {
            &[PropertyKey::FontAttributes, PropertyKey::LineAttributes]
        };
        for &key in keys {
            let Some(id) = get_object_property(object, key)
                .ok()
                .and_then(|value| value.as_u64())
            else {
                continue;
            };
            if let Some(count) = u16::try_from(id).ok().and_then(|id| uses.get_mut(&id)) {
                *count += 1;
            }
        }
    }
    uses.into_iter()
        .max_by_key(|&(id, count)| (count, std::cmp::Reverse(id)))
        .and_then(|(id, _)| ObjectId::new(id).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EditorProject;
    use ag_iso_stack::object_pool::vt_version::VtVersion;

    const PRESET_TYPES: [ObjectType; 21] = [
        ObjectType::Container,
        ObjectType::Animation,
        ObjectType::ScaledGraphic,
        ObjectType::Button,
        ObjectType::InputBoolean,
        ObjectType::InputString,
        ObjectType::OutputString,
        ObjectType::InputNumber,
        ObjectType::OutputNumber,
        ObjectType::InputList,
        ObjectType::OutputList,
        ObjectType::OutputLine,
        ObjectType::OutputRectangle,
        ObjectType::OutputEllipse,
        ObjectType::OutputPolygon,
        ObjectType::OutputMeter,
        ObjectType::OutputLinearBarGraph,
        ObjectType::OutputArchedBarGraph,
        ObjectType::LineAttributes,
        ObjectType::FontAttributes,
        ObjectType::DataMask,
    ];

    fn project() -> EditorProject {
        let mut project = EditorProject::default();
        project.set_vt_version(VtVersion::Version6).unwrap();
        project
    }

    fn create(project: &mut EditorProject, object_type: ObjectType) -> Object {
        let name = project.generate_smart_name_for_new_object(object_type);
        let id = project.create_object(object_type, name).unwrap();
        project.get_pool().object_by_id(id).unwrap().clone()
    }

    fn reference(object: &Object, property: PropertyKey) -> ObjectId {
        let id = get_object_property(object, property)
            .unwrap()
            .as_u64()
            .unwrap();
        ObjectId::new(id as u16).unwrap()
    }

    #[test]
    fn every_preset_is_applied() {
        for object_type in PRESET_TYPES {
            let mut project = project();
            let object = create(&mut project, object_type);
            for preset in presets(object_type) {
                match preset {
                    Preset::Value(property, value) => assert_eq!(
                        get_object_property(&object, property).unwrap(),
                        value,
                        "{object_type:?} {property}"
                    ),
                    Preset::Attribute(property, attribute_type) => {
                        let attribute = project
                            .get_pool()
                            .object_by_id(reference(&object, property))
                            .unwrap();
                        assert_eq!(attribute.object_type(), attribute_type, "{object_type:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn attributes_are_created_once_and_then_reused() {
        let mut project = project();
        let line = create(&mut project, ObjectType::OutputLine);
        let attribute = reference(&line, PropertyKey::LineAttributes);
        // The created Line Attributes draws a solid line
        let Some(Object::LineAttributes(attributes)) = project.get_pool().object_by_id(attribute)
        else {
            panic!("expected Line Attributes");
        };
        assert_eq!((attributes.line_width, attributes.line_art), (1, 0xFFFF));

        let rectangle = create(&mut project, ObjectType::OutputRectangle);
        assert_eq!(
            reference(&rectangle, PropertyKey::LineAttributes),
            attribute
        );
        assert_eq!(
            project
                .get_pool()
                .objects_by_type(ObjectType::LineAttributes)
                .len(),
            1
        );

        let string = create(&mut project, ObjectType::OutputString);
        let boolean = create(&mut project, ObjectType::InputBoolean);
        assert_eq!(
            reference(&boolean, PropertyKey::ForegroundColour),
            reference(&string, PropertyKey::FontAttributes)
        );
    }

    #[test]
    fn creation_with_presets_is_one_undo_step() {
        let mut project = project();
        let before = project.get_pool().objects().len();
        create(&mut project, ObjectType::OutputString);
        assert_eq!(project.get_pool().objects().len(), before + 2);
        project.undo();
        assert_eq!(project.get_pool().objects().len(), before);
    }
}

#[cfg(test)]
mod placement_tests {
    use crate::EditorProject;
    use ag_iso_stack::object_pool::{object::Object, ObjectType};

    #[test]
    fn create_object_at_places_the_object_on_its_parent_in_one_undo_step() {
        let mut project = EditorProject::default();
        let mask_name = project.generate_smart_name_for_new_object(ObjectType::DataMask);
        let mask = project
            .create_object(ObjectType::DataMask, mask_name)
            .unwrap();
        let before = project.get_pool().objects().len();

        let name = project.generate_smart_name_for_new_object(ObjectType::Button);
        let button = project
            .create_object_at(ObjectType::Button, name, mask, 12, 34)
            .unwrap();
        let Some(Object::DataMask(data_mask)) = project.get_pool().object_by_id(mask) else {
            panic!("expected the Data Mask");
        };
        let placed = data_mask.object_refs.last().unwrap();
        assert_eq!(
            (placed.id, placed.offset.x, placed.offset.y),
            (button, 12, 34)
        );

        project.undo();
        assert_eq!(project.get_pool().objects().len(), before);
        let Some(Object::DataMask(data_mask)) = project.get_pool().object_by_id(mask) else {
            panic!("expected the Data Mask");
        };
        assert!(data_mask.object_refs.is_empty());
    }
}
