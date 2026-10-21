//! Preview rendering for the numeric graphic objects in ISO 11783-6 B.11.

use ag_iso_stack::object_pool::{
    object::{NumberVariable, Object, OutputArchedBarGraph, OutputLinearBarGraph, OutputMeter},
    object_attributes::{AxisOrientation, BarGraphType, DeflectionDirection, GrowDirection, Point},
    ObjectPool,
};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke};

const ARC_STEPS: usize = 96;

pub(crate) fn render_meter(
    object: &OutputMeter,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
) {
    let rect = object_rect(ui, position, object.width as f32, object.width as f32);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.46;
    let arc_colour = colour(pool, object.arc_and_tick_colour);
    let border_colour = colour(pool, object.border_colour);
    let needle_colour = colour(pool, object.needle_colour);
    let (start, span) = angular_range(
        object.start_angle as f32 * 2.0,
        object.end_angle as f32 * 2.0,
        object.options.deflection_direction,
    );
    let clip = ui.clip_rect().intersect(rect);
    let painter = ui.painter().with_clip_rect(clip);

    if object.options.draw_arc {
        painter.add(egui::Shape::line(
            arc_points(center, radius, start, span),
            Stroke::new(1.0, arc_colour),
        ));
    }
    if object.options.draw_border {
        painter.add(egui::Shape::line(
            arc_points(center, radius, 0.0, std::f32::consts::TAU),
            Stroke::new(1.0, border_colour),
        ));
    }
    if object.options.draw_ticks && object.nr_of_ticks > 0 {
        for index in 0..object.nr_of_ticks as usize {
            let fraction = tick_fraction(index, object.nr_of_ticks as usize);
            let angle = start + span * fraction;
            let inner = point_on_arc(center, radius * 0.82, angle);
            let outer = point_on_arc(center, radius, angle);
            painter.line_segment([inner, outer], Stroke::new(1.0, arc_colour));
        }
    }

    if let Some(value) =
        variable_value(pool, object.variable_reference.0).or(Some(object.value as u32))
    {
        let min = object.min_value as u32;
        let max = object.max_value as u32;
        let Some(fraction) = (max > min)
            .then(|| value.clamp(min, max).saturating_sub(min) as f32 / (max - min) as f32)
        else {
            return;
        };
        let angle = start + span * fraction;
        painter.line_segment(
            [center, point_on_arc(center, radius * 0.78, angle)],
            Stroke::new(1.5, needle_colour),
        );
    }
}

pub(crate) fn render_linear_bar_graph(
    object: &OutputLinearBarGraph,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
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
    let value_position_fraction =
        value_fraction(value, object.min_value as u32, object.max_value as u32);

    let (axis_start, axis_end) = axis_points(rect, horizontal, grow_positive);
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
        let value_point = axis_start + (axis_end - axis_start) * fraction;
        if object.options.bar_graph_type == BarGraphType::Filled {
            let fill_rect = if horizontal {
                Rect::from_min_max(
                    Pos2::new(axis_start.x.min(value_point.x), rect.top()),
                    Pos2::new(axis_start.x.max(value_point.x), rect.bottom()),
                )
            } else {
                Rect::from_min_max(
                    Pos2::new(rect.left(), axis_start.y.min(value_point.y)),
                    Pos2::new(rect.right(), axis_start.y.max(value_point.y)),
                )
            };
            painter.rect_filled(fill_rect, 0.0, graph_colour);
        } else {
            let endpoints = if horizontal {
                [
                    Pos2::new(value_point.x, rect.top()),
                    Pos2::new(value_point.x, rect.bottom()),
                ]
            } else {
                [
                    Pos2::new(rect.left(), value_point.y),
                    Pos2::new(rect.right(), value_point.y),
                ]
            };
            painter.line_segment(endpoints, Stroke::new(1.5, graph_colour));
        }
    }
    if object.options.draw_target_line {
        let target = variable_value(pool, object.target_value_variable_reference.0)
            .unwrap_or(object.target_value as u32);
        let fraction = value_fraction(target, object.min_value as u32, object.max_value as u32)
            .unwrap_or_else(|| {
                if target < object.min_value as u32 {
                    0.0
                } else {
                    1.0
                }
            });
        let point = axis_start + (axis_end - axis_start) * fraction;
        let endpoints = if horizontal {
            [
                Pos2::new(point.x, rect.top()),
                Pos2::new(point.x, rect.bottom()),
            ]
        } else {
            [
                Pos2::new(rect.left(), point.y),
                Pos2::new(rect.right(), point.y),
            ]
        };
        painter.line_segment(endpoints, Stroke::new(1.5, target_colour));
    }
    if object.options.draw_border {
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, graph_colour),
            egui::StrokeKind::Inside,
        );
    }
}

pub(crate) fn render_arched_bar_graph(
    object: &OutputArchedBarGraph,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
) {
    let rect = object_rect(ui, position, object.width as f32, object.height as f32);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let fill_colour = colour(pool, object.colour);
    let target_colour = colour(pool, object.target_line_colour);
    let outer_radius_x = rect.width() * 0.48;
    let outer_radius_y = rect.height() * 0.48;
    let thickness = (object.bar_graph_width as f32)
        .min(outer_radius_x.min(outer_radius_y) - 1.0)
        .max(0.0);
    let center = rect.center();
    // Part 6 assigns option bit 4 to deflection direction. The current
    // ag-iso-stack model parses that bit as `axis_orientation` for this object.
    let direction = match object.options.axis_orientation {
        AxisOrientation::Vertical => DeflectionDirection::AntiClockwise,
        AxisOrientation::Horizontal => DeflectionDirection::Clockwise,
    };
    let (start, span) = angular_range(
        object.start_angle as f32 * 2.0,
        object.end_angle as f32 * 2.0,
        direction,
    );
    let value = variable_value(pool, object.variable_reference.0).unwrap_or(object.value as u32);
    let value_progress = value_fraction(value, object.min_value as u32, object.max_value as u32);
    let inner_radius_x = (outer_radius_x - thickness).max(0.0);
    let inner_radius_y = (outer_radius_y - thickness).max(0.0);

    if let Some(progress) = value_progress {
        if object.options.bar_graph_type == BarGraphType::Filled {
            for quad in annular_sector_quads(
                center,
                inner_radius_x,
                inner_radius_y,
                outer_radius_x,
                outer_radius_y,
                start,
                span * progress,
            ) {
                painter.add(egui::Shape::convex_polygon(quad, fill_colour, Stroke::NONE));
            }
        } else {
            let angle = start + span * progress;
            painter.line_segment(
                [
                    ellipse_point(center, inner_radius_x, inner_radius_y, angle),
                    ellipse_point(center, outer_radius_x, outer_radius_y, angle),
                ],
                Stroke::new(1.5, fill_colour),
            );
        }
    }
    if object.options.draw_target_line {
        let target = variable_value(pool, object.target_value_variable_reference.0)
            .unwrap_or(object.target_value as u32);
        let progress = value_fraction(target, object.min_value as u32, object.max_value as u32)
            .unwrap_or_else(|| {
                if target < object.min_value as u32 {
                    0.0
                } else {
                    1.0
                }
            });
        let angle = start + span * progress;
        painter.line_segment(
            [
                ellipse_point(center, inner_radius_x, inner_radius_y, angle),
                ellipse_point(center, outer_radius_x, outer_radius_y, angle),
            ],
            Stroke::new(1.5, target_colour),
        );
    }
    if object.options.draw_border {
        painter.add(egui::Shape::line(
            ellipse_arc_points(center, outer_radius_x, outer_radius_y, start, span),
            Stroke::new(1.0, fill_colour),
        ));
        painter.add(egui::Shape::line(
            ellipse_arc_points(center, inner_radius_x, inner_radius_y, start, span),
            Stroke::new(1.0, fill_colour),
        ));
        for angle in [start, start + span] {
            painter.line_segment(
                [
                    ellipse_point(center, inner_radius_x, inner_radius_y, angle),
                    ellipse_point(center, outer_radius_x, outer_radius_y, angle),
                ],
                Stroke::new(1.0, fill_colour),
            );
        }
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

fn value_fraction(value: u32, min: u32, max: u32) -> Option<f32> {
    if max <= min || value < min || value > max {
        None
    } else {
        Some((value - min) as f32 / (max - min) as f32)
    }
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

fn angular_range(start: f32, end: f32, direction: DeflectionDirection) -> (f32, f32) {
    let start = start.to_radians();
    let end = end.to_radians();
    let raw = end - start;
    let span = match direction {
        DeflectionDirection::AntiClockwise => raw.rem_euclid(std::f32::consts::TAU),
        DeflectionDirection::Clockwise => -(-raw).rem_euclid(std::f32::consts::TAU),
    };
    (
        start,
        if span.abs() < f32::EPSILON {
            direction_sign(direction) * std::f32::consts::TAU
        } else {
            span
        },
    )
}

fn direction_sign(direction: DeflectionDirection) -> f32 {
    if direction == DeflectionDirection::AntiClockwise {
        1.0
    } else {
        -1.0
    }
}

fn point_on_arc(center: Pos2, radius: f32, angle: f32) -> Pos2 {
    ellipse_point(center, radius, radius, angle)
}

fn ellipse_point(center: Pos2, radius_x: f32, radius_y: f32, angle: f32) -> Pos2 {
    center + egui::vec2(angle.cos() * radius_x, -angle.sin() * radius_y)
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
    fn value_range_excludes_out_of_range_values_and_handles_single_tick() {
        assert_eq!(value_fraction(15, 10, 20), Some(0.5));
        assert_eq!(value_fraction(9, 10, 20), None);
        assert_eq!(value_fraction(10, 10, 10), None);
        assert_eq!(tick_fraction(0, 1), 0.5);
    }

    #[test]
    fn angular_ranges_follow_direction_and_treat_equal_angles_as_full_circle() {
        let (start, ccw) = angular_range(0.0, 180.0, DeflectionDirection::AntiClockwise);
        let (_, cw) = angular_range(0.0, 180.0, DeflectionDirection::Clockwise);
        let (_, full) = angular_range(45.0, 45.0, DeflectionDirection::Clockwise);
        assert!(ccw > 0.0);
        assert!(cw < 0.0);
        assert!((full.abs() - std::f32::consts::TAU).abs() < 0.0001);
        assert_eq!(start, 0.0);
    }

    #[test]
    fn arched_sector_has_outer_and_inner_boundaries() {
        let quads = annular_sector_quads(
            Pos2::ZERO,
            5.0,
            4.0,
            10.0,
            8.0,
            0.0,
            std::f32::consts::FRAC_PI_2,
        );
        assert!(!quads.is_empty());
        assert_eq!(quads[0][0], ellipse_point(Pos2::ZERO, 10.0, 8.0, 0.0));
        assert_eq!(quads[0][3], ellipse_point(Pos2::ZERO, 5.0, 4.0, 0.0));
    }

    #[test]
    fn meter_preview_emits_graphics() {
        let ctx = egui::Context::default();
        let pool = ObjectPool::new();
        let meter = OutputMeter {
            id: ObjectId::new(1).unwrap(),
            width: 80,
            needle_colour: 12,
            border_colour: 1,
            arc_and_tick_colour: 0,
            options: OutputMeterOptions {
                draw_arc: true,
                draw_border: true,
                draw_ticks: true,
                deflection_direction: DeflectionDirection::AntiClockwise,
            },
            nr_of_ticks: 3,
            start_angle: 0,
            end_angle: 90,
            min_value: 0,
            max_value: 100,
            variable_reference: Default::default(),
            value: 50,
            macro_refs: vec![],
        };
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_meter(&meter, ui, &pool, Point { x: 0, y: 0 });
            });
        });
        assert!(!output.shapes.is_empty());
    }
}
