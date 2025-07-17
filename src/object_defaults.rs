//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::vec;

use ag_iso_stack::{
    network_management::name::NAME,
    object_pool::{object::*, object_attributes::*, NullableObjectId, ObjectId, ObjectType},
};

pub fn default_object(obj_type: ObjectType) -> Object {
    match obj_type {
        ObjectType::WorkingSet => Object::WorkingSet(default_working_set()),
        ObjectType::DataMask => Object::DataMask(default_data_mask()),
        ObjectType::AlarmMask => Object::AlarmMask(default_alarm_mask()),
        ObjectType::Container => Object::Container(default_container()),
        ObjectType::SoftKeyMask => Object::SoftKeyMask(default_soft_key_mask()),
        ObjectType::Key => Object::Key(default_key()),
        ObjectType::Button => Object::Button(default_button()),
        ObjectType::InputBoolean => Object::InputBoolean(default_input_boolean()),
        ObjectType::InputString => Object::InputString(default_input_string()),
        ObjectType::InputNumber => Object::InputNumber(default_input_number()),
        ObjectType::InputList => Object::InputList(default_input_list()),
        ObjectType::OutputString => Object::OutputString(default_output_string()),
        ObjectType::OutputNumber => Object::OutputNumber(default_output_number()),
        ObjectType::OutputLine => Object::OutputLine(default_output_line()),
        ObjectType::OutputRectangle => Object::OutputRectangle(default_output_rectangle()),
        ObjectType::OutputEllipse => Object::OutputEllipse(default_output_ellipse()),
        ObjectType::OutputPolygon => Object::OutputPolygon(default_output_polygon()),
        ObjectType::OutputMeter => Object::OutputMeter(default_output_meter()),
        ObjectType::OutputLinearBarGraph => {
            Object::OutputLinearBarGraph(default_output_linear_bar_graph())
        }
        ObjectType::OutputArchedBarGraph => {
            Object::OutputArchedBarGraph(default_output_arched_bar_graph())
        }
        ObjectType::PictureGraphic => Object::PictureGraphic(default_picture_graphic()),
        ObjectType::NumberVariable => Object::NumberVariable(default_number_variable()),
        ObjectType::StringVariable => Object::StringVariable(default_string_variable()),
        ObjectType::FontAttributes => Object::FontAttributes(default_font_attributes()),
        ObjectType::LineAttributes => Object::LineAttributes(default_line_attributes()),
        ObjectType::FillAttributes => Object::FillAttributes(default_fill_attributes()),
        ObjectType::InputAttributes => Object::InputAttributes(default_input_attributes()),
        ObjectType::ObjectPointer => Object::ObjectPointer(default_object_pointer()),
        ObjectType::Macro => Object::Macro(default_macro()),
        ObjectType::AuxiliaryFunctionType1 => {
            Object::AuxiliaryFunctionType1(default_auxiliary_function_type1())
        }
        ObjectType::AuxiliaryInputType1 => {
            Object::AuxiliaryInputType1(default_auxiliary_input_type1())
        }
        ObjectType::AuxiliaryFunctionType2 => {
            Object::AuxiliaryFunctionType2(default_auxiliary_function_type2())
        }
        ObjectType::AuxiliaryInputType2 => {
            Object::AuxiliaryInputType2(default_auxiliary_input_type2())
        }
        ObjectType::AuxiliaryControlDesignatorType2 => {
            Object::AuxiliaryControlDesignatorType2(default_auxiliary_control_designator_type2())
        }
        ObjectType::WindowMask => Object::WindowMask(default_window_mask()),
        ObjectType::KeyGroup => Object::KeyGroup(default_key_group()),
        ObjectType::GraphicsContext => Object::GraphicsContext(default_graphics_context()),
        ObjectType::OutputList => Object::OutputList(default_output_list()),
        ObjectType::ExtendedInputAttributes => {
            Object::ExtendedInputAttributes(default_extended_input_attributes())
        }
        ObjectType::ColourMap => Object::ColourMap(default_colour_map()),
        ObjectType::ObjectLabelReferenceList => {
            Object::ObjectLabelReferenceList(default_object_label_reference_list())
        }
        ObjectType::ExternalObjectDefinition => {
            Object::ExternalObjectDefinition(default_external_object_definition())
        }
        ObjectType::ExternalReferenceName => {
            Object::ExternalReferenceName(default_external_reference_name())
        }
        ObjectType::ExternalObjectPointer => {
            Object::ExternalObjectPointer(default_external_object_pointer())
        }
        ObjectType::Animation => Object::Animation(default_animation()),
        ObjectType::ColourPalette => Object::ColourPalette(default_colour_palette()),
        ObjectType::GraphicData => Object::GraphicData(default_graphic_data()),
        ObjectType::WorkingSetSpecialControls => {
            Object::WorkingSetSpecialControls(default_working_set_special_controls())
        }
        ObjectType::ScaledGraphic => Object::ScaledGraphic(default_scaled_graphic()),
    }
}

fn default_working_set() -> WorkingSet {
    WorkingSet {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        selectable: true,
        active_mask: ObjectId::new(256).unwrap(),
        object_refs: vec![],
        macro_refs: vec![],
        language_codes: vec![],
    }
}

fn default_data_mask() -> DataMask {
    DataMask {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        soft_key_mask: NullableObjectId::NULL,
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_alarm_mask() -> AlarmMask {
    AlarmMask {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        soft_key_mask: NullableObjectId::NULL,
        priority: 0,
        acoustic_signal: 0,
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_container() -> Container {
    Container {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        hidden: false,
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_soft_key_mask() -> SoftKeyMask {
    SoftKeyMask {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        objects: vec![],
        macro_refs: vec![],
    }
}

fn default_key() -> Key {
    Key {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        key_code: 0,
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_button() -> Button {
    Button {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        background_colour: 0,
        border_colour: 0,
        key_code: 0,
        options: ButtonOptions {
            latchable: false,
            state: ButtonState::Latched,
            suppress_border: false,
            transparent_background: false,
            disabled: false,
            no_border: false,
        },
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_input_boolean() -> InputBoolean {
    InputBoolean {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        width: 0,
        foreground_colour: ObjectId::new(256).unwrap(),
        variable_reference: NullableObjectId::NULL,
        value: false,
        enabled: true,
        macro_refs: vec![],
    }
}

fn default_input_string() -> InputString {
    InputString {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        background_colour: 0,
        font_attributes: ObjectId::new(256).unwrap(),
        input_attributes: NullableObjectId::NULL,
        options: InputStringOptions {
            transparent: false,
            auto_wrap: false,
            wrap_on_hyphen: false,
        },
        variable_reference: NullableObjectId::NULL,
        justification: Alignment {
            horizontal: HorizontalAlignment::Left,
            vertical: VerticalAlignment::Top,
        },
        value: "".to_string(),
        enabled: true,
        macro_refs: vec![],
    }
}

fn default_input_number() -> InputNumber {
    InputNumber {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        background_colour: 0,
        font_attributes: ObjectId::new(256).unwrap(),
        options: NumberOptions {
            transparent: false,
            display_leading_zeros: false,
            display_zero_as_blank: false,
            truncate: false,
        },
        variable_reference: NullableObjectId::NULL,
        value: 0,
        min_value: 0,
        max_value: u32::MAX,
        offset: 0,
        scale: 1.0,
        nr_of_decimals: 0,
        format: FormatType::Decimal,
        justification: Alignment {
            horizontal: HorizontalAlignment::Left,
            vertical: VerticalAlignment::Top,
        },
        options2: InputNumberOptions {
            enabled: false,
            real_time_editing: false,
        },
        macro_refs: vec![],
    }
}

fn default_input_list() -> InputList {
    InputList {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        variable_reference: NullableObjectId::NULL,
        value: 0,
        options: InputListOptions {
            enabled: false,
            real_time_editing: false,
        },
        list_items: vec![],
        macro_refs: vec![],
    }
}

fn default_output_string() -> OutputString {
    OutputString {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        background_colour: 0,
        font_attributes: ObjectId::new(256).unwrap(),
        options: OutputStringOptions {
            transparent: false,
            auto_wrap: false,
            wrap_on_hyphen: false,
        },
        variable_reference: NullableObjectId::NULL,
        justification: Alignment {
            horizontal: HorizontalAlignment::Left,
            vertical: VerticalAlignment::Top,
        },
        value: "".to_string(),
        macro_refs: vec![],
    }
}

fn default_output_number() -> OutputNumber {
    OutputNumber {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        background_colour: 0,
        font_attributes: ObjectId::new(256).unwrap(),
        options: NumberOptions {
            transparent: false,
            display_leading_zeros: false,
            display_zero_as_blank: false,
            truncate: false,
        },
        variable_reference: NullableObjectId::NULL,
        value: 0,
        offset: 0,
        scale: 0.0,
        nr_of_decimals: 0,
        format: FormatType::Decimal,
        justification: Alignment {
            horizontal: HorizontalAlignment::Left,
            vertical: VerticalAlignment::Top,
        },
        macro_refs: vec![],
    }
}

fn default_output_line() -> OutputLine {
    OutputLine {
        id: ObjectId::new(256).unwrap(),
        line_attributes: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        line_direction: LineDirection::TopLeftToBottomRight,
        macro_refs: vec![],
    }
}

fn default_output_rectangle() -> OutputRectangle {
    OutputRectangle {
        id: ObjectId::new(256).unwrap(),
        line_attributes: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        line_suppression: 0,
        fill_attributes: NullableObjectId::NULL,
        macro_refs: vec![],
    }
}

fn default_output_ellipse() -> OutputEllipse {
    OutputEllipse {
        id: ObjectId::new(256).unwrap(),
        line_attributes: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        ellipse_type: 0,
        start_angle: 0,
        end_angle: 0,
        fill_attributes: NullableObjectId::NULL,
        macro_refs: vec![],
    }
}

fn default_output_polygon() -> OutputPolygon {
    OutputPolygon {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        line_attributes: ObjectId::new(256).unwrap(),
        fill_attributes: NullableObjectId::NULL,
        polygon_type: 0,
        points: vec![
            Point { x: 0, y: 0 },
            Point { x: 0, y: 0 },
            Point { x: 0, y: 0 },
        ],
        macro_refs: vec![],
    }
}

fn default_output_meter() -> OutputMeter {
    OutputMeter {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        needle_colour: 0,
        border_colour: 0,
        arc_and_tick_colour: 0,
        options: OutputMeterOptions {
            draw_arc: false,
            draw_border: false,
            draw_ticks: false,
            deflection_direction: DeflectionDirection::Clockwise,
        },
        nr_of_ticks: 0,
        start_angle: 0,
        end_angle: 0,
        min_value: 0,
        max_value: 0,
        variable_reference: NullableObjectId::NULL,
        value: 0,
        macro_refs: vec![],
    }
}

fn default_output_linear_bar_graph() -> OutputLinearBarGraph {
    OutputLinearBarGraph {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        colour: 0,
        target_line_colour: 0,
        options: OutputLinearBarGraphOptions {
            draw_border: false,
            draw_target_line: false,
            draw_ticks: false,
            bar_graph_type: BarGraphType::Filled,
            axis_orientation: AxisOrientation::Horizontal,
            grow_direction: GrowDirection::GrowRightUp,
        },
        nr_of_ticks: 0,
        min_value: 0,
        max_value: 0,
        variable_reference: NullableObjectId::NULL,
        value: 0,
        target_value_variable_reference: NullableObjectId::NULL,
        target_value: 0,
        macro_refs: vec![],
    }
}

fn default_output_arched_bar_graph() -> OutputArchedBarGraph {
    OutputArchedBarGraph {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        colour: 0,
        target_line_colour: 0,
        options: OutputArchedBarGraphOptions {
            draw_border: false,
            draw_target_line: false,
            bar_graph_type: BarGraphType::Filled,
            axis_orientation: AxisOrientation::Horizontal,
            grow_direction: GrowDirection::GrowRightUp,
            deflection_direction: DeflectionDirection::Clockwise,
        },
        start_angle: 0,
        end_angle: 0,
        bar_graph_width: 0,
        min_value: 0,
        max_value: 0,
        variable_reference: NullableObjectId::NULL,
        value: 0,
        target_value_variable_reference: NullableObjectId::NULL,
        target_value: 0,
        macro_refs: vec![],
    }
}

fn default_picture_graphic() -> PictureGraphic {
    PictureGraphic {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        actual_width: 0,
        actual_height: 0,
        format: PictureGraphicFormat::Monochrome,
        options: PictureGraphicOptions {
            transparent: false,
            flashing: false,
            data_code_type: DataCodeType::Raw,
        },
        transparency_colour: 0,
        data: vec![],
        macro_refs: vec![],
    }
}

fn default_number_variable() -> NumberVariable {
    NumberVariable {
        id: ObjectId::new(256).unwrap(),
        value: 0,
    }
}

fn default_string_variable() -> StringVariable {
    StringVariable {
        id: ObjectId::new(256).unwrap(),
        value: "".to_string(),
    }
}

fn default_font_attributes() -> FontAttributes {
    FontAttributes {
        id: ObjectId::new(256).unwrap(),
        font_colour: 0,
        font_size: FontSize::NonProportional(NonProportionalFontSize::Px6x8),
        font_type: FontType::Latin1,
        font_style: FontStyle {
            bold: false,
            crossed_out: false,
            underlined: false,
            italic: false,
            inverted: false,
            flashing_inverted: false,
            flashing_hidden: false,
            proportional: false,
        },
        macro_refs: vec![],
    }
}

fn default_line_attributes() -> LineAttributes {
    LineAttributes {
        id: ObjectId::new(256).unwrap(),
        line_colour: 0,
        line_width: 0,
        line_art: 0,
        macro_refs: vec![],
    }
}

fn default_fill_attributes() -> FillAttributes {
    FillAttributes {
        id: ObjectId::new(256).unwrap(),
        fill_type: 0,
        fill_colour: 0,
        fill_pattern: NullableObjectId::NULL,
        macro_refs: vec![],
    }
}

fn default_input_attributes() -> InputAttributes {
    InputAttributes {
        id: ObjectId::new(256).unwrap(),
        validation_type: ValidationType::ValidCharacters,
        validation_string: "".to_string(),
        macro_refs: vec![],
    }
}

fn default_object_pointer() -> ObjectPointer {
    ObjectPointer {
        id: ObjectId::new(256).unwrap(),
        value: NullableObjectId::NULL,
    }
}

fn default_macro() -> Macro {
    Macro {
        id: ObjectId::new(0).unwrap(),
        commands: vec![],
    }
}

fn default_auxiliary_function_type1() -> AuxiliaryFunctionType1 {
    AuxiliaryFunctionType1 {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        function_type: 0,
        object_refs: vec![],
    }
}

fn default_auxiliary_input_type1() -> AuxiliaryInputType1 {
    AuxiliaryInputType1 {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        function_type: 0,
        input_id: 0,
        object_refs: vec![],
    }
}

fn default_auxiliary_function_type2() -> AuxiliaryFunctionType2 {
    AuxiliaryFunctionType2 {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        function_attributes: FunctionAttributes {
            function_type: AuxiliaryFunctionType::BooleanLatching,
            critical: false,
            restricted: false,
            single_assignment: false,
        },
        object_refs: vec![],
    }
}

fn default_auxiliary_input_type2() -> AuxiliaryInputType2 {
    AuxiliaryInputType2 {
        id: ObjectId::new(256).unwrap(),
        background_colour: 0,
        function_attributes: FunctionAttributes {
            function_type: AuxiliaryFunctionType::BooleanLatching,
            critical: false,
            restricted: false,
            single_assignment: false,
        },
        object_refs: vec![],
    }
}

fn default_auxiliary_control_designator_type2() -> AuxiliaryControlDesignatorType2 {
    AuxiliaryControlDesignatorType2 {
        id: ObjectId::new(256).unwrap(),
        pointer_type: 0,
        auxiliary_object_id: NullableObjectId::NULL,
    }
}

fn default_window_mask() -> WindowMask {
    WindowMask {
        id: ObjectId::new(256).unwrap(),
        cell_format: WindowMaskCellFormat::CF1x1,
        window_type: WindowType::FreeForm,
        background_colour: 0,
        options: WindowMaskOptions {
            available: true,
            transparent: false,
        },
        name: ObjectId::new(256).unwrap(),
        window_title: NullableObjectId::NULL,
        window_icon: NullableObjectId::NULL,
        objects: vec![],
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_key_group() -> KeyGroup {
    KeyGroup {
        id: ObjectId::new(256).unwrap(),
        options: KeyGroupOptions {
            available: true,
            transparent: false,
        },
        name: ObjectId::new(256).unwrap(),
        key_group_icon: NullableObjectId::NULL,
        objects: vec![],
        macro_refs: vec![],
    }
}

fn default_graphics_context() -> GraphicsContext {
    GraphicsContext {
        id: ObjectId::new(256).unwrap(),
        viewport_width: 0,
        viewport_height: 0,
        viewport_x: 0,
        viewport_y: 0,
        canvas_width: 0,
        canvas_height: 0,
        viewport_zoom: 0.0,
        graphics_cursor_x: 0,
        graphics_cursor_y: 0,
        foreground_colour: 0,
        background_colour: 0,
        font_attributes_object: NullableObjectId::NULL,
        line_attributes_object: NullableObjectId::NULL,
        fill_attributes_object: NullableObjectId::NULL,
        format: ColorFormat::Color8Bit,
        options: GraphicsContextOptions {
            transparent: false,
            color: ColorOption::ForegroundBackground,
        },
        transparency_colour: 0,
    }
}

fn default_output_list() -> OutputList {
    OutputList {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        variable_reference: NullableObjectId::NULL,
        value: 0,
        list_items: vec![],
        macro_refs: vec![],
    }
}

fn default_extended_input_attributes() -> ExtendedInputAttributes {
    ExtendedInputAttributes {
        id: ObjectId::new(256).unwrap(),
        validation_type: ValidationType::ValidCharacters,
        code_planes: vec![],
    }
}

fn default_colour_map() -> ColourMap {
    ColourMap {
        id: ObjectId::new(256).unwrap(),
        colour_map: vec![],
    }
}

fn default_object_label_reference_list() -> ObjectLabelReferenceList {
    ObjectLabelReferenceList {
        id: ObjectId::new(256).unwrap(),
        object_labels: vec![],
    }
}

fn default_external_object_definition() -> ExternalObjectDefinition {
    ExternalObjectDefinition {
        id: ObjectId::new(256).unwrap(),
        options: ExternalObjectDefinitionOptions { enabled: true },
        name: NAME::default(),
        objects: vec![],
    }
}

fn default_external_reference_name() -> ExternalReferenceName {
    ExternalReferenceName {
        id: ObjectId::new(256).unwrap(),
        options: ExternalReferenceNameOptions { enabled: true },
        name: NAME::default(),
    }
}

fn default_external_object_pointer() -> ExternalObjectPointer {
    ExternalObjectPointer {
        id: ObjectId::new(256).unwrap(),
        default_object_id: NullableObjectId::NULL,
        external_reference_name_id: NullableObjectId::NULL,
        external_object_id: NullableObjectId::NULL,
    }
}

fn default_animation() -> Animation {
    Animation {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        refresh_interval: 0,
        value: 0,
        enabled: true,
        first_child_index: 0,
        last_child_index: 0,
        default_child_index: 0,
        options: AnimationOptions {
            animation_sequence: AnimationSequence::Loop,
            disabled_behaviour: DisabledBehaviour::Pause,
        },
        object_refs: vec![],
        macro_refs: vec![],
    }
}

fn default_colour_palette() -> ColourPalette {
    ColourPalette {
        id: ObjectId::new(256).unwrap(),
        options: ColourPaletteOptions {},
        colours: vec![],
    }
}

fn default_graphic_data() -> GraphicData {
    GraphicData {
        id: ObjectId::new(256).unwrap(),
        format: 0,
        data: vec![],
    }
}

fn default_working_set_special_controls() -> WorkingSetSpecialControls {
    WorkingSetSpecialControls {
        id: ObjectId::new(256).unwrap(),
        id_of_colour_map: NullableObjectId::NULL,
        id_of_colour_palette: NullableObjectId::NULL,
        language_pairs: vec![],
    }
}

fn default_scaled_graphic() -> ScaledGraphic {
    ScaledGraphic {
        id: ObjectId::new(256).unwrap(),
        width: 0,
        height: 0,
        scale_type: 0,
        options: ScaledGraphicOptions { flashing: false },
        value: NullableObjectId::NULL,
        macro_refs: vec![],
    }
}
