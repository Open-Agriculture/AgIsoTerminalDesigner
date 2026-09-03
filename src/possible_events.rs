//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::{object::*, object_attributes::Event};

pub trait PossibleEvents {
    fn get_possible_events() -> Vec<Event>;
}

/// Return the macro events supported by an object without making UI code know
/// about each concrete object structure.
pub fn get_possible_events(object: &Object) -> Vec<Event> {
    match object {
        Object::WorkingSet(_) => WorkingSet::get_possible_events(),
        Object::DataMask(_) => DataMask::get_possible_events(),
        Object::AlarmMask(_) => AlarmMask::get_possible_events(),
        Object::Container(_) => Container::get_possible_events(),
        Object::SoftKeyMask(_) => SoftKeyMask::get_possible_events(),
        Object::Key(_) => Key::get_possible_events(),
        Object::Button(_) => Button::get_possible_events(),
        Object::InputBoolean(_) => InputBoolean::get_possible_events(),
        Object::InputString(_) => InputString::get_possible_events(),
        Object::InputNumber(_) => InputNumber::get_possible_events(),
        Object::InputList(_) => InputList::get_possible_events(),
        Object::OutputString(_) => OutputString::get_possible_events(),
        Object::OutputNumber(_) => OutputNumber::get_possible_events(),
        Object::OutputList(_) => OutputList::get_possible_events(),
        Object::OutputLine(_) => OutputLine::get_possible_events(),
        Object::OutputRectangle(_) => OutputRectangle::get_possible_events(),
        Object::OutputEllipse(_) => OutputEllipse::get_possible_events(),
        Object::OutputPolygon(_) => OutputPolygon::get_possible_events(),
        Object::OutputMeter(_) => OutputMeter::get_possible_events(),
        Object::OutputLinearBarGraph(_) => OutputLinearBarGraph::get_possible_events(),
        Object::OutputArchedBarGraph(_) => OutputArchedBarGraph::get_possible_events(),
        Object::PictureGraphic(_) => PictureGraphic::get_possible_events(),
        Object::NumberVariable(_) => NumberVariable::get_possible_events(),
        Object::StringVariable(_) => StringVariable::get_possible_events(),
        Object::FontAttributes(_) => FontAttributes::get_possible_events(),
        Object::LineAttributes(_) => LineAttributes::get_possible_events(),
        Object::FillAttributes(_) => FillAttributes::get_possible_events(),
        Object::InputAttributes(_) => InputAttributes::get_possible_events(),
        Object::ObjectPointer(_) => ObjectPointer::get_possible_events(),
        Object::GraphicsContext(_) => GraphicsContext::get_possible_events(),
        Object::KeyGroup(_) => KeyGroup::get_possible_events(),
        Object::ExternalObjectDefinition(_) => ExternalObjectDefinition::get_possible_events(),
        Object::WindowMask(_) => WindowMask::get_possible_events(),
        Object::ExternalReferenceName(_) => ExternalReferenceName::get_possible_events(),
        Object::ExternalObjectPointer(_) => ExternalObjectPointer::get_possible_events(),
        Object::Animation(_) => Animation::get_possible_events(),
        Object::ScaledGraphic(_) => ScaledGraphic::get_possible_events(),
        _ => Vec::new(),
    }
}

impl PossibleEvents for WorkingSet {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnActivate,
            Event::OnDeactivate,
            Event::OnChangeActiveMask,
            Event::OnChangeBackgroundColour,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
        ]
    }
}

impl PossibleEvents for DataMask {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnShow,
            Event::OnHide,
            // Event::OnRefresh,
            Event::OnChangeBackgroundColour,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
            Event::OnChangeSoftKeyMask,
            Event::OnChangeAttribute,
            Event::OnPointingEventPress,
            Event::OnPointingEventRelease,
        ]
    }
}

impl PossibleEvents for AlarmMask {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnShow,
            Event::OnHide,
            // Event::OnRefresh,
            Event::OnChangeBackgroundColour,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
            Event::OnChangePriority,
            Event::OnChangeSoftKeyMask,
            Event::OnChangeAttribute,
        ]
    }
}

impl PossibleEvents for Container {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnShow,
            Event::OnHide,
            // Event::OnRefresh,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for SoftKeyMask {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnShow,
            Event::OnHide,
            Event::OnChangeBackgroundColour,
            Event::OnChangeAttribute,
        ]
    }
}

impl PossibleEvents for Key {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnKeyPress,
            Event::OnKeyRelease,
            Event::OnChangeBackgroundColour,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
            Event::OnChangeAttribute,
            Event::OnInputFieldSelection,
            Event::OnInputFieldDeselection,
        ]
    }
}

impl PossibleEvents for Button {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnEnable,
            Event::OnDisable,
            Event::OnInputFieldSelection,
            Event::OnInputFieldDeselection,
            Event::OnKeyPress,
            Event::OnKeyRelease,
            Event::OnChangeBackgroundColour,
            Event::OnChangeSize,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
            Event::OnChangeAttribute,
        ]
    }
}

impl PossibleEvents for InputBoolean {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnEnable,
            Event::OnDisable,
            Event::OnInputFieldSelection,
            Event::OnInputFieldDeselection,
            Event::OnESC,
            Event::OnChangeBackgroundColour,
            Event::OnChangeValue,
            Event::OnEntryOfValue,
            Event::OnEntryOfNewValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for InputString {
    fn get_possible_events() -> Vec<Event> {
        InputBoolean::get_possible_events()
    }
}

impl PossibleEvents for InputNumber {
    fn get_possible_events() -> Vec<Event> {
        InputBoolean::get_possible_events()
    }
}

impl PossibleEvents for InputList {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnEnable,
            Event::OnDisable,
            Event::OnInputFieldSelection,
            Event::OnInputFieldDeselection,
            Event::OnESC,
            Event::OnChangeValue,
            Event::OnEntryOfValue,
            Event::OnEntryOfNewValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputString {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeBackgroundColour,
            Event::OnChangeValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputNumber {
    fn get_possible_events() -> Vec<Event> {
        // The events are the same for the output field objects, so we can use the list defined in OutputString
        OutputString::get_possible_events()
    }
}

impl PossibleEvents for OutputList {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputLine {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeEndPoint,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputRectangle {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeSize,
            Event::OnChangeAttribute,
        ]
    }
}

impl PossibleEvents for OutputEllipse {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeSize,
            Event::OnChangeAttribute,
        ]
    }
}

impl PossibleEvents for OutputPolygon {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputMeter {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputLinearBarGraph {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for OutputArchedBarGraph {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for PictureGraphic {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeAttribute,
        ]
    }
}

impl PossibleEvents for NumberVariable {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeValue]
    }
}

impl PossibleEvents for StringVariable {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeValue]
    }
}

impl PossibleEvents for FontAttributes {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeFontAttributes, Event::OnChangeAttribute]
    }
}

impl PossibleEvents for LineAttributes {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeLineAttributes, Event::OnChangeAttribute]
    }
}

impl PossibleEvents for FillAttributes {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeFillAttributes, Event::OnChangeAttribute]
    }
}

impl PossibleEvents for InputAttributes {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeValue]
    }
}

impl PossibleEvents for ObjectPointer {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeValue]
    }
}

impl PossibleEvents for GraphicsContext {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeAttribute, Event::OnChangeBackgroundColour]
    }
}

impl PossibleEvents for KeyGroup {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeAttribute]
    }
}

impl PossibleEvents for ExternalObjectDefinition {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeAttribute]
    }
}

impl PossibleEvents for WindowMask {
    fn get_possible_events() -> Vec<Event> {
        vec![
            Event::OnShow,
            Event::OnHide,
            // Event::OnRefresh,
            Event::OnChangeBackgroundColour,
            Event::OnChangeChildLocation,
            Event::OnChangeChildPosition,
            Event::OnChangeAttribute,
            Event::OnPointingEventPress,
            Event::OnPointingEventRelease,
        ]
    }
}

impl PossibleEvents for ExternalReferenceName {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeAttribute]
    }
}

impl PossibleEvents for ExternalObjectPointer {
    fn get_possible_events() -> Vec<Event> {
        vec![Event::OnChangeValue]
    }
}

impl PossibleEvents for Animation {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnEnable,
            Event::OnDisable,
            Event::OnChangeValue,
            Event::OnChangeAttribute,
            Event::OnChangeSize,
        ]
    }
}

impl PossibleEvents for ScaledGraphic {
    fn get_possible_events() -> Vec<Event> {
        vec![
            // Event::OnRefresh,
            Event::OnChangeAttribute,
            Event::OnChangeValue,
        ]
    }
}
