//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::{
    object::*, object_attributes::ObjectLabel, vt_version::VtVersion, ObjectType,
};

pub fn get_allowed_child_refs(obj_type: ObjectType, version: VtVersion) -> Vec<ObjectType> {
    let mut allowed = match obj_type {
        ObjectType::WorkingSet => WorkingSet::get_allowed_child_refs(version),
        ObjectType::DataMask => DataMask::get_allowed_child_refs(version),
        ObjectType::AlarmMask => AlarmMask::get_allowed_child_refs(version),
        ObjectType::Container => Container::get_allowed_child_refs(version),
        ObjectType::SoftKeyMask => SoftKeyMask::get_allowed_child_refs(version),
        ObjectType::Key => Key::get_allowed_child_refs(version),
        ObjectType::Button => Button::get_allowed_child_refs(version),
        ObjectType::InputList => InputList::get_allowed_child_refs(version),
        ObjectType::OutputList => OutputList::get_allowed_child_refs(version),
        ObjectType::AuxiliaryFunctionType1 => {
            AuxiliaryFunctionType1::get_allowed_child_refs(version)
        }
        ObjectType::AuxiliaryInputType1 => AuxiliaryInputType1::get_allowed_child_refs(version),
        ObjectType::AuxiliaryFunctionType2 => {
            AuxiliaryFunctionType2::get_allowed_child_refs(version)
        }
        ObjectType::AuxiliaryInputType2 => AuxiliaryInputType2::get_allowed_child_refs(version),
        ObjectType::WindowMask => WindowMask::get_allowed_child_refs(version),
        ObjectType::KeyGroup => KeyGroup::get_allowed_child_refs(version),
        ObjectType::Animation => Animation::get_allowed_child_refs(version),
        ObjectType::ObjectLabelReferenceList => {
            ObjectLabelReferenceList::get_allowed_child_refs(version)
        }
        _ => vec![],
    };
    allowed
        .retain(|child_type| crate::vt_compatibility::supported_object_type(*child_type, version));
    allowed
}

pub trait AllowedChildRefs {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType>;
}

/// Add the Type 2 auxiliary objects defined for one parent by Table A.2
/// (ISO 11783-6:2018, pp. 72–74). Function/input and designator introductions
/// differ for Window Mask, Input List and Output List parents.
fn append_type2_auxiliary_children(
    allowed: &mut Vec<ObjectType>,
    version: VtVersion,
    function_input_since: VtVersion,
    designator_since: VtVersion,
) {
    if version >= function_input_since {
        allowed.extend_from_slice(&[
            ObjectType::AuxiliaryFunctionType2,
            ObjectType::AuxiliaryInputType2,
        ]);
    }
    if version >= designator_since {
        allowed.push(ObjectType::AuxiliaryControlDesignatorType2);
    }
}

impl AllowedChildRefs for WorkingSet {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![
            ObjectType::OutputString,
            ObjectType::OutputNumber,
            ObjectType::OutputLine,
            ObjectType::OutputRectangle,
            ObjectType::OutputEllipse,
            ObjectType::OutputPolygon,
            ObjectType::PictureGraphic,
        ];
        if version >= VtVersion::Version4 {
            allowed_objects.extend_from_slice(&[
                ObjectType::OutputList,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
                ObjectType::ObjectPointer,
            ]);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }

        allowed_objects
    }
}

impl AllowedChildRefs for DataMask {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![
            ObjectType::Container,
            ObjectType::Button,
            ObjectType::InputBoolean,
            ObjectType::InputString,
            ObjectType::InputNumber,
            ObjectType::InputList,
            ObjectType::OutputString,
            ObjectType::OutputNumber,
            ObjectType::OutputLine,
            ObjectType::OutputRectangle,
            ObjectType::OutputEllipse,
            ObjectType::OutputPolygon,
            ObjectType::OutputMeter,
            ObjectType::OutputLinearBarGraph,
            ObjectType::OutputArchedBarGraph,
            ObjectType::PictureGraphic,
            ObjectType::ObjectPointer,
        ];
        if version >= VtVersion::Version3 {
            allowed_objects.push(ObjectType::WorkingSet);
        }
        if version >= VtVersion::Version4 {
            allowed_objects.push(ObjectType::OutputList);
            allowed_objects.push(ObjectType::GraphicsContext);
        }
        if version >= VtVersion::Version5 {
            allowed_objects.push(ObjectType::Animation);
            allowed_objects.push(ObjectType::ExternalObjectPointer);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }
        append_type2_auxiliary_children(
            &mut allowed_objects,
            version,
            VtVersion::Version3,
            VtVersion::Version3,
        );

        allowed_objects
    }
}

impl AllowedChildRefs for AlarmMask {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![
            ObjectType::Container,
            ObjectType::OutputString,
            ObjectType::OutputNumber,
            ObjectType::OutputLine,
            ObjectType::OutputRectangle,
            ObjectType::OutputEllipse,
            ObjectType::OutputPolygon,
            ObjectType::OutputMeter,
            ObjectType::OutputLinearBarGraph,
            ObjectType::OutputArchedBarGraph,
            ObjectType::PictureGraphic,
            ObjectType::ObjectPointer,
        ];
        if version >= VtVersion::Version3 {
            allowed_objects.push(ObjectType::WorkingSet);
        }
        if version >= VtVersion::Version4 {
            allowed_objects.push(ObjectType::OutputList);
            allowed_objects.push(ObjectType::GraphicsContext);
        }
        if version >= VtVersion::Version5 {
            allowed_objects.push(ObjectType::Animation);
            allowed_objects.push(ObjectType::ExternalObjectPointer);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }
        append_type2_auxiliary_children(
            &mut allowed_objects,
            version,
            VtVersion::Version3,
            VtVersion::Version3,
        );

        allowed_objects
    }
}

impl AllowedChildRefs for Container {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        // As of VT version 6, the same objects are allowed in a container as in a data mask
        DataMask::get_allowed_child_refs(version)
    }
}

impl AllowedChildRefs for SoftKeyMask {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![ObjectType::Key, ObjectType::ObjectPointer];
        if version >= VtVersion::Version5 {
            allowed_objects.push(ObjectType::ExternalObjectPointer);
        }

        allowed_objects
    }
}

impl AllowedChildRefs for Key {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![
            ObjectType::Container,
            ObjectType::OutputString,
            ObjectType::OutputNumber,
            ObjectType::OutputLine,
            ObjectType::OutputRectangle,
            ObjectType::OutputEllipse,
            ObjectType::OutputPolygon,
            ObjectType::PictureGraphic,
            ObjectType::ObjectPointer,
        ];
        if version >= VtVersion::Version4 {
            allowed_objects.extend_from_slice(&[
                ObjectType::WorkingSet,
                ObjectType::OutputList,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
            ]);
        }
        if version >= VtVersion::Version5 {
            allowed_objects.push(ObjectType::Animation);
            allowed_objects.push(ObjectType::ExternalObjectPointer);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }

        allowed_objects
    }
}

impl AllowedChildRefs for Button {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        // As of VT version 6, the same objects are allowed in a button as in a key object
        Key::get_allowed_child_refs(version)
    }
}

impl AllowedChildRefs for InputList {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![
            ObjectType::OutputString,
            ObjectType::OutputNumber,
            ObjectType::PictureGraphic,
        ];
        if version >= VtVersion::Version4 {
            allowed_objects.extend_from_slice(&[
                ObjectType::WorkingSet,
                ObjectType::Container,
                ObjectType::OutputList,
                ObjectType::OutputLine,
                ObjectType::OutputRectangle,
                ObjectType::OutputEllipse,
                ObjectType::OutputPolygon,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
                ObjectType::ObjectPointer,
            ]);
        }
        if version >= VtVersion::Version5 {
            allowed_objects.push(ObjectType::ExternalObjectPointer);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }
        append_type2_auxiliary_children(
            &mut allowed_objects,
            version,
            VtVersion::Version6,
            VtVersion::Version4,
        );

        allowed_objects
    }
}

impl AllowedChildRefs for OutputList {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        // Table A.2 gives Output List the same VT6/VT4 boundaries as Window Mask
        // for Auxiliary Function/Input Type 2 and its designator pointer.
        WindowMask::get_allowed_child_refs(version)
    }
}

impl AllowedChildRefs for AuxiliaryFunctionType1 {
    fn get_allowed_child_refs(_: VtVersion) -> Vec<ObjectType> {
        let allowed_objects = vec![
            ObjectType::OutputString,
            ObjectType::OutputNumber,
            ObjectType::OutputLine,
            ObjectType::OutputRectangle,
            ObjectType::OutputEllipse,
            ObjectType::OutputPolygon,
            ObjectType::PictureGraphic,
        ];
        allowed_objects
    }
}

impl AllowedChildRefs for AuxiliaryInputType1 {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        // The same objects are allowed in an auxiliary input type 1 as in an auxiliary function type 1
        AuxiliaryFunctionType1::get_allowed_child_refs(version)
    }
}

impl AllowedChildRefs for AuxiliaryFunctionType2 {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![];
        if version >= VtVersion::Version3 {
            allowed_objects.extend_from_slice(&[
                ObjectType::Container,
                ObjectType::OutputString,
                ObjectType::OutputNumber,
                ObjectType::OutputLine,
                ObjectType::OutputRectangle,
                ObjectType::OutputEllipse,
                ObjectType::OutputPolygon,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::PictureGraphic,
                ObjectType::ObjectPointer,
            ]);
        }
        if version >= VtVersion::Version4 {
            allowed_objects.push(ObjectType::OutputList);
            allowed_objects.push(ObjectType::GraphicsContext);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }
        allowed_objects
    }
}

impl AllowedChildRefs for AuxiliaryInputType2 {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        // The same objects are allowed in an auxiliary input type 2 as in an auxiliary function type 2
        AuxiliaryFunctionType2::get_allowed_child_refs(version)
    }
}

impl AllowedChildRefs for WindowMask {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![];
        if version >= VtVersion::Version4 {
            allowed_objects.extend_from_slice(&[
                ObjectType::WorkingSet,
                ObjectType::Container,
                ObjectType::Button,
                ObjectType::InputBoolean,
                ObjectType::InputString,
                ObjectType::InputNumber,
                ObjectType::InputList,
                ObjectType::OutputString,
                ObjectType::OutputNumber,
                ObjectType::OutputList,
                ObjectType::OutputLine,
                ObjectType::OutputRectangle,
                ObjectType::OutputEllipse,
                ObjectType::OutputPolygon,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
                ObjectType::PictureGraphic,
                ObjectType::ObjectPointer,
            ]);
        }
        if version >= VtVersion::Version5 {
            allowed_objects.push(ObjectType::Animation);
            allowed_objects.push(ObjectType::ExternalObjectPointer);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }
        append_type2_auxiliary_children(
            &mut allowed_objects,
            version,
            VtVersion::Version6,
            VtVersion::Version4,
        );

        allowed_objects
    }
}

impl AllowedChildRefs for KeyGroup {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![];
        if version >= VtVersion::Version4 {
            allowed_objects.push(ObjectType::Key);
        }

        allowed_objects
    }
}

impl AllowedChildRefs for Animation {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![];
        if version >= VtVersion::Version5 {
            allowed_objects.extend_from_slice(&[
                ObjectType::Container,
                ObjectType::OutputString,
                ObjectType::OutputNumber,
                ObjectType::OutputList,
                ObjectType::OutputLine,
                ObjectType::OutputRectangle,
                ObjectType::OutputEllipse,
                ObjectType::OutputPolygon,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
                ObjectType::PictureGraphic,
                ObjectType::ObjectPointer,
            ]);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }

        allowed_objects
    }
}

impl AllowedChildRefs for ObjectLabelReferenceList {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        // Object Label Reference List is a reference set, not a visual
        // container. Table A.2 (pp. 72–74) defines which pool objects may be
        // addressed by entries in the list.
        let mut allowed_objects = vec![];
        if version >= VtVersion::Version4 {
            allowed_objects.extend_from_slice(&[
                ObjectType::WorkingSet,
                ObjectType::DataMask,
                ObjectType::AlarmMask,
                ObjectType::Container,
                ObjectType::WindowMask,
                ObjectType::SoftKeyMask,
                ObjectType::Key,
                ObjectType::Button,
                ObjectType::KeyGroup,
                ObjectType::InputBoolean,
                ObjectType::InputString,
                ObjectType::InputNumber,
                ObjectType::InputList,
                ObjectType::OutputString,
                ObjectType::OutputNumber,
                ObjectType::OutputList,
                ObjectType::OutputLine,
                ObjectType::OutputRectangle,
                ObjectType::OutputEllipse,
                ObjectType::OutputPolygon,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
                ObjectType::PictureGraphic,
            ]);
            // Type 2 function/input objects are valid list references from VT4;
            // their designator pointer is permitted from VT5 (Table A.2, p. 74).
            allowed_objects.extend_from_slice(&[
                ObjectType::AuxiliaryFunctionType2,
                ObjectType::AuxiliaryInputType2,
            ]);
        }
        if version >= VtVersion::Version5 {
            allowed_objects.extend_from_slice(&[
                ObjectType::Animation,
                ObjectType::NumberVariable,
                ObjectType::StringVariable,
                ObjectType::FontAttributes,
                ObjectType::LineAttributes,
                ObjectType::FillAttributes,
                ObjectType::InputAttributes,
                ObjectType::ExtendedInputAttributes,
                ObjectType::ColourMap,
                ObjectType::ObjectLabelReferenceList,
                ObjectType::ObjectPointer,
                ObjectType::ExternalObjectDefinition,
                ObjectType::ExternalReferenceName,
                ObjectType::ExternalObjectPointer,
                ObjectType::Macro,
                ObjectType::AuxiliaryFunctionType1,
                ObjectType::AuxiliaryInputType1,
                ObjectType::AuxiliaryControlDesignatorType2,
            ]);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.extend_from_slice(&[
                ObjectType::ScaledGraphic,
                ObjectType::ColourPalette,
                ObjectType::GraphicData,
                ObjectType::WorkingSetSpecialControls,
            ]);
        }

        allowed_objects
    }
}

impl AllowedChildRefs for ObjectLabel {
    fn get_allowed_child_refs(version: VtVersion) -> Vec<ObjectType> {
        let mut allowed_objects = vec![];
        if version >= VtVersion::Version4 {
            allowed_objects.extend_from_slice(&[
                ObjectType::Container,
                ObjectType::OutputString,
                ObjectType::OutputNumber,
                ObjectType::OutputList,
                ObjectType::OutputLine,
                ObjectType::OutputRectangle,
                ObjectType::OutputEllipse,
                ObjectType::OutputPolygon,
                ObjectType::OutputMeter,
                ObjectType::OutputLinearBarGraph,
                ObjectType::OutputArchedBarGraph,
                ObjectType::GraphicsContext,
                ObjectType::PictureGraphic,
                ObjectType::ObjectPointer,
            ]);
        }
        if version >= VtVersion::Version6 {
            allowed_objects.push(ObjectType::ScaledGraphic);
        }

        allowed_objects
    }
}
