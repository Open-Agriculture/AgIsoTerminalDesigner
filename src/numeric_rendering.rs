//! Preview rendering for the numeric graphic objects in ISO 11783-6 B.11.

use ag_iso_stack::object_pool::{
    object::{NumberVariable, Object, OutputArchedBarGraph, OutputLinearBarGraph, OutputMeter},
    object_attributes::{AxisOrientation, BarGraphType, DeflectionDirection, GrowDirection, Point},
    vt_version::VtVersion,
    ObjectPool,
};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};

const ARC_STEPS: usize = 96;

pub(crate) fn render_meter(
    object: &OutputMeter,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
    version: VtVersion,
) {
    let rect = object_rect(ui, position, object.width as f32, object.width as f32);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let center = rect.center();
    // The meter's circle fills its square, half a pixel in so the arc stays inside it
    let radius = (rect.width() / 2.0 - 0.5).max(0.0);
    let arc_colour = colour(pool, object.arc_and_tick_colour);
    let border_colour = colour(pool, object.border_colour);
    let needle_colour = colour(pool, object.needle_colour);
    let (start, span) = deflection_range(
        object.start_angle as f32 * 2.0,
        object.end_angle as f32 * 2.0,
        object.options.deflection_direction,
    );
    let clip = ui.clip_rect().intersect(rect);
    let painter = ui.painter().with_clip_rect(clip);

    if object.options.draw_border {
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, border_colour),
            egui::StrokeKind::Inside,
        );
    }
    if object.options.draw_arc {
        painter.add(egui::Shape::line(
            arc_points(center, radius, start, span),
            Stroke::new(1.0, arc_colour),
        ));
    }
    if object.options.draw_ticks && object.nr_of_ticks > 0 {
        let tick_length = (rect.width() / 10.0).max(1.0);
        for index in 0..object.nr_of_ticks as usize {
            let fraction = tick_fraction(index, object.nr_of_ticks as usize);
            let angle = start + span * fraction;
            let inner = point_on_arc(center, (radius - tick_length).max(0.0), angle);
            let outer = point_on_arc(center, radius, angle);
            painter.line_segment([inner, outer], Stroke::new(1.0, arc_colour));
        }
    }

    let value = variable_value(pool, object.variable_reference.0).unwrap_or(object.value as u32);
    let min = object.min_value as u32;
    let max = object.max_value as u32;
    // Preserve the historical meter endpoint clamping on VT2/VT3.
    let value = if max > min {
        value.clamp(min, max)
    } else {
        value
    };
    if let Some(fraction) = value_fraction(value, min, max, version) {
        let angle = start + span * fraction;
        painter.line_segment(
            [center, point_on_arc(center, radius, angle)],
            Stroke::new(3.0, needle_colour),
        );
    }
}

pub(crate) fn render_linear_bar_graph(
    object: &OutputLinearBarGraph,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
    version: VtVersion,
) {
    let rect = object_rect(ui, position, object.width as f32, object.height as f32);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let graph_colour = colour(pool, object.colour);
    let target_colour = colour(pool, object.target_line_colour);
    let horizontal = object.options.axis_orientation == AxisOrientation::Horizontal;
    let grow_positive = object.options.grow_direction == GrowDirection::GrowRightUp;
    let value = variable_value(pool, object.variable_reference.0).unwrap_or(object.value as u32);
    let value_position_fraction = value_fraction(
        value,
        object.min_value as u32,
        object.max_value as u32,
        version,
    );

    let (axis_start, axis_end) = axis_points(rect, horizontal, grow_positive);
    let length = if horizontal {
        rect.width()
    } else {
        rect.height()
    };
    // The bar, the value line and the target line are drawn over the border
    if object.options.draw_border {
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(3.0, graph_colour),
            egui::StrokeKind::Inside,
        );
    }
    if object.options.draw_ticks && object.nr_of_ticks > 0 {
        for index in 0..object.nr_of_ticks as usize {
            let fraction = tick_fraction(index, object.nr_of_ticks as usize);
            let p = axis_start + (axis_end - axis_start) * fraction;
            let (a, b) = if horizontal {
                (p + egui::vec2(0.0, -3.0), p + egui::vec2(0.0, 3.0))
            } else {
                (p + egui::vec2(-3.0, 0.0), p + egui::vec2(3.0, 0.0))
            };
            painter.line_segment([a, b], Stroke::new(1.0, graph_colour));
        }
    }
    if let Some(fraction) = value_position_fraction {
        let band = if object.options.bar_graph_type == BarGraphType::Filled {
            linear_band(rect, horizontal, grow_positive, 0.0, fraction * length)
        } else {
            // Not filled: the value is a 3 pixel line that stays inside the bar at both ends
            let line_width = length.min(3.0);
            linear_band(
                rect,
                horizontal,
                grow_positive,
                fraction * (length - line_width),
                line_width,
            )
        };
        painter.rect_filled(band, 0.0, graph_colour);
    }
    if object.options.draw_target_line {
        let target = variable_value(pool, object.target_value_variable_reference.0)
            .unwrap_or(object.target_value as u32);
        let fraction = target_fraction(
            target,
            object.min_value as u32,
            object.max_value as u32,
            version,
        );
        let line_width = length.min(1.0);
        painter.rect_filled(
            linear_band(
                rect,
                horizontal,
                grow_positive,
                fraction * (length - line_width),
                line_width,
            ),
            0.0,
            target_colour,
        );
    }
}

pub(crate) fn render_arched_bar_graph(
    object: &OutputArchedBarGraph,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
    version: VtVersion,
) {
    let rect = object_rect(ui, position, object.width as f32, object.height as f32);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let fill_colour = colour(pool, object.colour);
    let target_colour = colour(pool, object.target_line_colour);
    let center = rect.center();
    // The graph is drawn on the ellipse that fills the object's rectangle, half a pixel in so
    // the border stays inside it
    let outer_radius_x = (rect.width() / 2.0 - 0.5).max(0.0);
    let outer_radius_y = (rect.height() / 2.0 - 0.5).max(0.0);
    // A thinner bar than requested is drawn when it would not fit
    let thickness = (object.bar_graph_width as f32)
        .min(outer_radius_x.min(outer_radius_y))
        .max(1.0);
    // Part 6 assigns option bit 4 to deflection direction. The current
    // ag-iso-stack model parses that bit as `axis_orientation` for this object.
    let direction = match object.options.axis_orientation {
        AxisOrientation::Vertical => DeflectionDirection::AntiClockwise,
        AxisOrientation::Horizontal => DeflectionDirection::Clockwise,
    };
    let (start, span) = deflection_range(
        object.start_angle as f32 * 2.0,
        object.end_angle as f32 * 2.0,
        direction,
    );
    let value = variable_value(pool, object.variable_reference.0).unwrap_or(object.value as u32);
    let value_progress = value_fraction(
        value,
        object.min_value as u32,
        object.max_value as u32,
        version,
    );
    let inner_radius_x = (outer_radius_x - thickness).max(0.0);
    let inner_radius_y = (outer_radius_y - thickness).max(0.0);
    let radial_line = |angle: f32, width: f32, colour: Color32| {
        painter.line_segment(
            [
                ellipse_point(center, inner_radius_x, inner_radius_y, angle),
                ellipse_point(center, outer_radius_x, outer_radius_y, angle),
            ],
            Stroke::new(width, colour),
        );
    };

    if let Some(progress) = value_progress {
        let value_angle = start + span * progress;
        if object.options.bar_graph_type == BarGraphType::Filled {
            // Fill anticlockwise from the lower angle so every quad winds the same way
            let (from, sweep) = if span < 0.0 {
                (value_angle, -span * progress)
            } else {
                (start, span * progress)
            };
            for quad in annular_sector_quads(
                center,
                inner_radius_x,
                inner_radius_y,
                outer_radius_x,
                outer_radius_y,
                from,
                sweep,
            ) {
                painter.add(egui::Shape::convex_polygon(quad, fill_colour, Stroke::NONE));
            }
        } else {
            // Not filled: the value is a line across the bar
            radial_line(value_angle, 3.0, fill_colour);
        }
    }
    if object.options.draw_border {
        let full_ring = span.abs() >= std::f32::consts::TAU;
        let (from, sweep) = if span < 0.0 {
            (start + span, -span)
        } else {
            (start, span)
        };
        for (radius_x, radius_y) in [
            (outer_radius_x, outer_radius_y),
            (inner_radius_x, inner_radius_y),
        ] {
            painter.add(egui::Shape::line(
                ellipse_arc_points(center, radius_x, radius_y, from, sweep),
                Stroke::new(1.0, fill_colour),
            ));
        }
        // A closed ring has no ends to draw
        if !full_ring {
            for angle in [from, from + sweep] {
                radial_line(angle, 1.0, fill_colour);
            }
        }
    }
    if object.options.draw_target_line {
        let target = variable_value(pool, object.target_value_variable_reference.0)
            .unwrap_or(object.target_value as u32);
        let progress = target_fraction(
            target,
            object.min_value as u32,
            object.max_value as u32,
            version,
        );
        radial_line(start + span * progress, 1.0, target_colour);
    }
}

fn object_rect(ui: &egui::Ui, position: Point<i16>, width: f32, height: f32) -> Rect {
    let min = ui.min_rect().min + egui::vec2(position.x as f32, position.y as f32);
    Rect::from_min_size(min, egui::vec2(width, height))
}

fn colour(pool: &ObjectPool, index: u8) -> Color32 {
    let [r, g, b, a] = crate::effective_colour::colour(pool, index).as_rgba();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn variable_value(
    pool: &ObjectPool,
    id: Option<ag_iso_stack::object_pool::ObjectId>,
) -> Option<u32> {
    id.and_then(|id| pool.object_by_id(id))
        .and_then(|object| match object {
            Object::NumberVariable(NumberVariable { value, .. }) => Some(*value),
            _ => None,
        })
}

/// B.11.1 defines endpoint saturation and degenerate ranges from VT4 onward.
fn value_fraction(value: u32, min: u32, max: u32, version: VtVersion) -> Option<f32> {
    if version >= VtVersion::Version4 {
        return Some(if max <= min {
            0.0
        } else {
            (value.clamp(min, max) - min) as f32 / (max - min) as f32
        });
    }
    if max <= min || value < min || value > max {
        None
    } else {
        Some((value - min) as f32 / (max - min) as f32)
    }
}

fn target_fraction(value: u32, min: u32, max: u32, version: VtVersion) -> f32 {
    value_fraction(value, min, max, version).unwrap_or(if value < min { 0.0 } else { 1.0 })
}

fn tick_fraction(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.5
    } else {
        index as f32 / (count - 1) as f32
    }
}

fn axis_points(rect: Rect, horizontal: bool, positive: bool) -> (Pos2, Pos2) {
    match (horizontal, positive) {
        (true, true) => (rect.left_center(), rect.right_center()),
        (true, false) => (rect.right_center(), rect.left_center()),
        (false, true) => (
            Pos2::new(rect.center().x, rect.bottom()),
            Pos2::new(rect.center().x, rect.top()),
        ),
        (false, false) => (
            Pos2::new(rect.center().x, rect.top()),
            Pos2::new(rect.center().x, rect.bottom()),
        ),
    }
}

/// Returns the angle of the minimum value and the signed span to the maximum value.
///
/// The arc always runs anticlockwise from the start angle to the end angle, equal
/// angles meaning a full circle. The deflection direction only picks the end the
/// value grows from: a clockwise value starts at the end angle, which is how real
/// pools draw a 0 to 180 degree clockwise gauge over the top.
fn deflection_range(start: f32, end: f32, direction: DeflectionDirection) -> (f32, f32) {
    let start = start.to_radians();
    let mut sweep = (end.to_radians() - start).rem_euclid(std::f32::consts::TAU);
    if sweep < f32::EPSILON {
        sweep = std::f32::consts::TAU;
    }
    match direction {
        DeflectionDirection::AntiClockwise => (start, sweep),
        DeflectionDirection::Clockwise => (start + sweep, -sweep),
    }
}

/// The band of a linear bar graph `size` pixels thick, `distance` pixels from its minimum end.
fn linear_band(
    rect: Rect,
    horizontal: bool,
    grows_positive: bool,
    distance: f32,
    size: f32,
) -> Rect {
    match (horizontal, grows_positive) {
        (true, true) => Rect::from_min_size(
            Pos2::new(rect.left() + distance, rect.top()),
            egui::vec2(size, rect.height()),
        ),
        (true, false) => Rect::from_min_size(
            Pos2::new(rect.right() - distance - size, rect.top()),
            egui::vec2(size, rect.height()),
        ),
        (false, true) => Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() - distance - size),
            egui::vec2(rect.width(), size),
        ),
        (false, false) => Rect::from_min_size(
            Pos2::new(rect.left(), rect.top() + distance),
            egui::vec2(rect.width(), size),
        ),
    }
}

fn point_on_arc(center: Pos2, radius: f32, angle: f32) -> Pos2 {
    ellipse_point(center, radius, radius, angle)
}

fn ellipse_point(center: Pos2, radius_x: f32, radius_y: f32, angle: f32) -> Pos2 {
    // Intersect a ray at the specified geometric angle with the ellipse.
    // Scaling a circle's parameter coordinates distorts angles (Figure B.8).
    if radius_x <= 0.0 || radius_y <= 0.0 {
        return center;
    }
    let direction = egui::vec2(angle.cos(), -angle.sin());
    let distance =
        1.0 / ((direction.x / radius_x).powi(2) + (direction.y / radius_y).powi(2)).sqrt();
    center + direction * distance
}

fn arc_points(center: Pos2, radius: f32, start: f32, span: f32) -> Vec<Pos2> {
    ellipse_arc_points(center, radius, radius, start, span)
}

fn ellipse_arc_points(
    center: Pos2,
    radius_x: f32,
    radius_y: f32,
    start: f32,
    span: f32,
) -> Vec<Pos2> {
    let segments = ((span.abs() / std::f32::consts::TAU * ARC_STEPS as f32).ceil() as usize).max(1);
    (0..=segments)
        .map(|index| {
            ellipse_point(
                center,
                radius_x,
                radius_y,
                start + span * index as f32 / segments as f32,
            )
        })
        .collect()
}

fn annular_sector_quads(
    center: Pos2,
    inner_x: f32,
    inner_y: f32,
    outer_x: f32,
    outer_y: f32,
    start: f32,
    span: f32,
) -> Vec<Vec<Pos2>> {
    if span.abs() < f32::EPSILON {
        return vec![];
    }
    let outer_points = ellipse_arc_points(center, outer_x, outer_y, start, span);
    let inner_points = ellipse_arc_points(center, inner_x, inner_y, start, span);
    outer_points
        .windows(2)
        .zip(inner_points.windows(2))
        .map(|(outer, inner)| vec![outer[0], outer[1], inner[1], inner[0]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object_attributes::OutputMeterOptions, ObjectId};

    #[test]
    fn value_ranges_follow_version_rules_and_handle_single_tick() {
        for version in [VtVersion::Version2, VtVersion::Version3] {
            assert_eq!(value_fraction(15, 10, 20, version), Some(0.5));
            assert_eq!(value_fraction(9, 10, 20, version), None);
            assert_eq!(value_fraction(21, 10, 20, version), None);
            assert_eq!(value_fraction(10, 10, 10, version), None);
            assert_eq!(target_fraction(21, 10, 20, version), 1.0);
        }
        for version in [
            VtVersion::Version4,
            VtVersion::Version5,
            VtVersion::Version6,
        ] {
            assert_eq!(value_fraction(15, 10, 20, version), Some(0.5));
            assert_eq!(value_fraction(9, 10, 20, version), Some(0.0));
            assert_eq!(value_fraction(21, 10, 20, version), Some(1.0));
            for value in [0, 10, 20, u32::MAX] {
                assert_eq!(value_fraction(value, 10, 10, version), Some(0.0));
                assert_eq!(value_fraction(value, 20, 10, version), Some(0.0));
                assert_eq!(target_fraction(value, 10, 10, version), 0.0);
                assert_eq!(target_fraction(value, 20, 10, version), 0.0);
            }
        }
        assert_eq!(tick_fraction(0, 1), 0.5);
    }

    #[test]
    fn deflection_ranges_share_the_arc_and_treat_equal_angles_as_full_circle() {
        use std::f32::consts::{PI, TAU};
        let (start, ccw) = deflection_range(0.0, 180.0, DeflectionDirection::AntiClockwise);
        let (cw_start, cw) = deflection_range(0.0, 180.0, DeflectionDirection::Clockwise);
        let (_, full) = deflection_range(45.0, 45.0, DeflectionDirection::Clockwise);
        assert_eq!(start, 0.0);
        assert!((ccw - PI).abs() < 0.0001);
        // A clockwise value starts at the end angle and sweeps back over the same arc
        assert!((cw_start - PI).abs() < 0.0001);
        assert!((cw + PI).abs() < 0.0001);
        assert!((full.abs() - TAU).abs() < 0.0001);
    }

    #[test]
    fn clockwise_meter_needle_sweeps_over_the_top() {
        let pool = ObjectPool::new();
        let mut object = crate::default_object(ag_iso_stack::object_pool::ObjectType::OutputMeter);
        let Object::OutputMeter(meter) = &mut object else {
            unreachable!()
        };
        meter.width = 80;
        meter.needle_colour = 12;
        meter.options = OutputMeterOptions {
            draw_arc: false,
            draw_border: false,
            draw_ticks: false,
            deflection_direction: DeflectionDirection::Clockwise,
        };
        meter.start_angle = 0;
        meter.end_angle = 90;
        meter.min_value = 0;
        meter.max_value = 100;
        meter.variable_reference = ag_iso_stack::object_pool::NullableObjectId::NULL;
        for (value, check) in [
            (0, (|a: Pos2, b: Pos2| b.x < a.x) as fn(Pos2, Pos2) -> bool),
            (50, |a: Pos2, b: Pos2| b.y < a.y),
            (100, |a: Pos2, b: Pos2| b.x > a.x),
        ] {
            if let Object::OutputMeter(meter) = &mut object {
                meter.value = value;
            }
            let output = render(&object, &pool, VtVersion::Version4);
            let needles = indicator_lines(&output, colour(&pool, 12));
            assert_eq!(needles.len(), 1, "{value}");
            let [a, b] = needles[0];
            assert!(check(a, b), "{value}: {a:?} -> {b:?}");
        }
    }

    #[test]
    fn arched_sector_has_outer_and_inner_boundaries() {
        let quads = annular_sector_quads(
            Pos2::ZERO,
            5.0,
            4.0,
            10.0,
            8.0,
            std::f32::consts::FRAC_PI_4,
            std::f32::consts::FRAC_PI_4,
        );
        assert!(!quads.is_empty());
        for (point, rx, ry) in [(quads[0][0], 10.0, 8.0), (quads[0][3], 5.0, 4.0)] {
            // Both boundaries meet the same geometric 45-degree ray.
            assert!((point.x + point.y).abs() < 0.0001);
            assert!(((point.x / rx).powi(2) + (point.y / ry).powi(2) - 1.0).abs() < 0.0001);
        }
    }

    fn render(object: &Object, pool: &ObjectPool, version: VtVersion) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::object_rendering::render_object_with_version(
                    ui,
                    pool,
                    object,
                    Point::default(),
                    version,
                );
            });
        })
    }

    fn indicator_lines(output: &egui::FullOutput, colour: Color32) -> Vec<[Pos2; 2]> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == colour => {
                    Some(*points)
                }
                _ => None,
            })
            .collect()
    }

    fn indicator_bands(output: &egui::FullOutput, colour: Color32) -> Vec<Rect> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == colour => Some(rect.rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn numeric_previews_draw_values_and_targets_at_endpoints() {
        use ag_iso_stack::object_pool::{NullableObjectId, ObjectType};
        let mut pool = ObjectPool::new();
        pool.add(Object::NumberVariable(NumberVariable {
            id: ObjectId::new(2).unwrap(),
            value: 200,
        }));
        for kind in [
            ObjectType::OutputMeter,
            ObjectType::OutputLinearBarGraph,
            ObjectType::OutputArchedBarGraph,
        ] {
            let mut object = crate::default_object(kind);
            let reference = NullableObjectId::new(2);
            match &mut object {
                Object::OutputMeter(o) => {
                    o.width = 80;
                    o.needle_colour = 12;
                    o.options = OutputMeterOptions {
                        draw_arc: false,
                        draw_border: false,
                        draw_ticks: false,
                        deflection_direction: DeflectionDirection::AntiClockwise,
                    };
                    o.start_angle = 0;
                    o.end_angle = 45;
                    o.min_value = 10;
                    o.max_value = 100;
                    o.variable_reference = reference;
                }
                Object::OutputLinearBarGraph(o) => {
                    o.width = 80;
                    o.height = 40;
                    o.colour = 12;
                    o.target_line_colour = 14;
                    o.options.draw_border = false;
                    o.options.draw_ticks = false;
                    o.options.draw_target_line = true;
                    o.options.bar_graph_type = BarGraphType::NotFilled;
                    o.options.axis_orientation = AxisOrientation::Horizontal;
                    o.options.grow_direction = GrowDirection::GrowRightUp;
                    o.min_value = 10;
                    o.max_value = 100;
                    o.variable_reference = reference;
                    o.target_value_variable_reference = reference;
                }
                Object::OutputArchedBarGraph(o) => {
                    o.width = 80;
                    o.height = 40;
                    o.bar_graph_width = 5;
                    o.colour = 12;
                    o.target_line_colour = 14;
                    o.options.draw_border = false;
                    o.options.draw_target_line = true;
                    o.options.bar_graph_type = BarGraphType::NotFilled;
                    o.options.axis_orientation = AxisOrientation::Vertical;
                    o.start_angle = 0;
                    o.end_angle = 45;
                    o.min_value = 10;
                    o.max_value = 100;
                    o.variable_reference = reference;
                    o.target_value_variable_reference = reference;
                }
                _ => unreachable!(),
            }
            for range in [(10, 100), (100, 100), (100, 10)] {
                match &mut object {
                    Object::OutputMeter(o) => {
                        o.min_value = range.0;
                        o.max_value = range.1;
                    }
                    Object::OutputLinearBarGraph(o) => {
                        o.min_value = range.0;
                        o.max_value = range.1;
                    }
                    Object::OutputArchedBarGraph(o) => {
                        o.min_value = range.0;
                        o.max_value = range.1;
                    }
                    _ => unreachable!(),
                }
                for raw in [0, 200] {
                    if let Some(Object::NumberVariable(v)) =
                        pool.object_mut_by_id(ObjectId::new(2).unwrap())
                    {
                        v.value = raw;
                    }
                    for version in [VtVersion::Version4, VtVersion::Version6] {
                        let output = render(&object, &pool, version);
                        let at_minimum = range.0 >= range.1 || raw == 0;
                        match kind {
                            ObjectType::OutputMeter | ObjectType::OutputArchedBarGraph => {
                                let values = indicator_lines(&output, colour(&pool, 12));
                                assert_eq!(values.len(), 1, "{kind:?}, {range:?}, {raw}");
                                let [a, b] = values[0];
                                if at_minimum {
                                    assert!(b.x > a.x && (a.y - b.y).abs() < 0.001);
                                } else {
                                    assert!(b.y < a.y && (a.x - b.x).abs() < 0.001);
                                }
                                if kind == ObjectType::OutputArchedBarGraph {
                                    assert_eq!(indicator_lines(&output, colour(&pool, 14)), values);
                                }
                            }
                            ObjectType::OutputLinearBarGraph => {
                                // The 3 pixel value line and 1 pixel target line stay inside
                                // the bar at both ends
                                for (index, width) in [(12, 3.0), (14, 1.0)] {
                                    let bands = indicator_bands(&output, colour(&pool, index));
                                    assert_eq!(bands.len(), 1, "{kind:?}, {range:?}, {raw}");
                                    let band = bands[0];
                                    assert!((band.width() - width).abs() < 0.001);
                                    if at_minimum {
                                        assert!((band.left() - 8.0).abs() < 0.001);
                                    } else {
                                        assert!((band.right() - 88.0).abs() < 0.001);
                                    }
                                }
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
        }
    }
}
