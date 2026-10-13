//! ISO 11783-6:2018 Output String presentation (§4.6.19 and Tables B.22/B.46).
use std::{collections::HashMap, sync::Arc, time::Duration};

use ag_iso_stack::object_pool::{
    object::{Object, OutputString},
    object_attributes::{FontSize, FontStyle, HorizontalAlignment, Point, VerticalAlignment},
    ObjectPool,
};
use egui::{Color32, FontFamily, FontId, Galley, Rect, Stroke};

use crate::object_rendering::Colorable;

/// Presentation-only normalization; never changes the pool's stored value.
fn controls(text: &str) -> Vec<Vec<char>> {
    let mut lines = vec![Vec::new()];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\0' => break,
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(Vec::new());
            }
            // Annex K control codes consume no space. Unsupported Unicode
            // formatting controls likewise must not advance the cursor.
            c if c.is_control()
                || matches!(c, '\u{feff}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}') =>
                {}
            c => lines.last_mut().unwrap().push(c),
        }
    }
    lines
}

fn justify(line: &mut Vec<char>, alignment: HorizontalAlignment) {
    if alignment == HorizontalAlignment::Middle {
        let first = line.iter().position(|c| *c != ' ').unwrap_or(line.len());
        line.drain(..first);
    }
    if matches!(
        alignment,
        HorizontalAlignment::Middle | HorizontalAlignment::Right
    ) {
        while line.last() == Some(&' ') {
            line.pop();
        }
    }
}

/// Layout keeps forced line ends distinct from auto-wraps (Table 6).
fn layout(
    paragraphs: Vec<Vec<char>>,
    width: f32,
    wrap: bool,
    hyphen: bool,
    alignment: HorizontalAlignment,
    advance: impl Fn(char) -> f32,
) -> Vec<Vec<char>> {
    let mut result = Vec::new();
    let paragraph_count = paragraphs.len();
    for (index, mut paragraph) in paragraphs.into_iter().enumerate() {
        justify(&mut paragraph, alignment);
        if !wrap || paragraph.is_empty() {
            result.push(paragraph.into_iter().filter(|c| *c != '\u{ad}').collect());
            continue;
        }
        let mut start = 0;
        while start < paragraph.len() {
            let mut end = start;
            let mut used = 0.0;
            let mut opportunity = None;
            while end < paragraph.len() {
                let c = paragraph[end];
                if c == '\u{ad}' {
                    if used + advance('-') <= width {
                        opportunity = Some((end + 1, true));
                    }
                    end += 1;
                    continue;
                }
                if c == ' ' && end > start {
                    opportunity = Some((end, false));
                }
                if used + advance(c) > width {
                    break;
                }
                used += advance(c);
                end += 1;
                if c == '-' && hyphen {
                    opportunity = Some((end, false));
                }
            }
            let overflowing = end < paragraph.len();
            let (next, soft) = if overflowing {
                opportunity.unwrap_or((end.max(start + 1), false))
            } else {
                (end, false)
            };
            let mut line: Vec<char> = paragraph[start..next]
                .iter()
                .copied()
                .filter(|c| *c != '\u{ad}')
                .collect();
            if soft {
                line.push('-');
            }
            justify(&mut line, alignment);
            result.push(line);
            start = next;
            if overflowing {
                while paragraph.get(start) == Some(&' ') {
                    start += 1;
                }
                // A wrap before residual spaces and a subsequent forced break
                // are two line ends (§4.6.19.5 Example 2).
                if start == paragraph.len() && index + 1 < paragraph_count {
                    result.push(Vec::new());
                }
            }
        }
    }
    result
}

struct Glyph {
    galley: Arc<Galley>,
    advance: f32,
}

/// Fit styled glyph geometry inside its character box. Texture coordinates stay
/// in atlas texels so egui can normalize them after all glyphs have been loaded.
fn styled_glyph(mut galley: Arc<Galley>, width: f32, height: f32, style: FontStyle) -> Arc<Galley> {
    let g = Arc::make_mut(&mut galley);
    let original_height = g.rect.height().max(1.0);
    for placed in &mut g.rows {
        let row = Arc::make_mut(&mut placed.row);
        let mesh = &mut row.visuals.mesh;
        let scale_y = height / original_height;
        let scale_x = width / row.size.x.max(1.0);
        for v in &mut mesh.vertices {
            v.pos.x *= scale_x;
            v.pos.y *= scale_y;
            if style.italic {
                v.pos.x += (height - v.pos.y) * 0.2;
            }
        }
        if style.bold {
            let mut extra = mesh.clone();
            for v in &mut extra.vertices {
                v.pos.x += (width * 0.08).min(1.0);
            }
            mesh.append(extra);
        }
        let bounds = mesh.calc_bounds();
        if bounds.is_positive() {
            // Preserve baseline spacing unless an overhang would escape the cell.
            let sx = (width / bounds.width()).min(1.0);
            let sy = (height / bounds.height()).min(1.0);
            let dx = (-bounds.left() * sx)
                .max(0.0)
                .min(width - bounds.right() * sx);
            let dy = (-bounds.top() * sy)
                .max(0.0)
                .min(height - bounds.bottom() * sy);
            for v in &mut mesh.vertices {
                v.pos.x = v.pos.x * sx + dx;
                v.pos.y = v.pos.y * sy + dy;
            }
        }
        row.size = egui::vec2(width, height);
        row.visuals.mesh_bounds = mesh.calc_bounds();
        placed.pos = egui::Pos2::ZERO;
    }
    g.rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
    g.mesh_bounds = g.rows.iter().fold(Rect::NOTHING, |bounds, r| {
        bounds.union(r.visuals.mesh_bounds)
    });
    g.num_vertices = g.rows.iter().map(|r| r.visuals.mesh.vertices.len()).sum();
    g.num_indices = g.rows.iter().map(|r| r.visuals.mesh.indices.len()).sum();
    galley
}

pub(crate) fn render(
    object: &OutputString,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
) {
    // Never shorten the layout rectangle at a parent edge: clipping cannot alter
    // wrapping or justification.
    let rect = Rect::from_min_size(
        ui.max_rect().min + egui::vec2(position.x as f32, position.y as f32),
        egui::vec2(object.width as f32, object.height as f32),
    );
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if !rect.is_positive() || !painter.clip_rect().is_positive() {
        return;
    }
    let error = |message: &str| {
        painter.text(
            rect.min,
            egui::Align2::LEFT_TOP,
            message,
            FontId::monospace(10.0),
            Color32::RED,
        );
    };
    let Some(Object::FontAttributes(font)) = pool.object_by_id(object.font_attributes) else {
        error("Missing Font Attributes");
        return;
    };
    if object.justification.horizontal == HorizontalAlignment::Reserved
        || object.justification.vertical == VerticalAlignment::Reserved
    {
        error("Reserved text justification");
        return;
    }
    let value = if let Some(id) = object.variable_reference.0 {
        let Some(Object::StringVariable(variable)) = pool.object_by_id(id) else {
            error("Missing String Variable");
            return;
        };
        &variable.value
    } else {
        &object.value
    };
    let text = match value.decode(font.font_type) {
        Ok(text) => text,
        Err(_) => {
            error("Unsupported string encoding");
            return;
        }
    };
    let style = font.font_style;
    let phase = ui.input(|i| i.time).rem_euclid(1.0);
    if style.flashing_hidden || style.flashing_inverted {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(if phase < 0.5 {
                0.5 - phase
            } else {
                1.0 - phase
            }));
    }
    if style.flashing_hidden && phase >= 0.5 {
        return;
    }
    let inverted = if !style.flashing_hidden && style.flashing_inverted {
        phase >= 0.5
    } else {
        style.inverted
    };
    let mut foreground = crate::effective_colour::colour(pool, font.font_colour).convert();
    let mut background = crate::effective_colour::colour(pool, object.background_colour).convert();
    if inverted {
        std::mem::swap(&mut foreground, &mut background);
    }
    if !object.options.transparent {
        painter.rect_filled(rect, 0.0, background);
    }
    let (height, cell_width, family) = match font.font_size {
        FontSize::NonProportional(size) => (
            size.height() as f32,
            Some(size.width() as f32),
            FontFamily::Monospace,
        ),
        FontSize::Proportional(height) => (height as f32, None, FontFamily::Proportional),
    };
    if height < 8.0 {
        error("Invalid font height");
        return;
    }
    let paragraphs = controls(&text);
    let mut glyphs = HashMap::new();
    for c in paragraphs.iter().flatten().copied().chain(['-']) {
        if c == '\u{ad}' || glyphs.contains_key(&c) {
            continue;
        }
        // These Greek punctuation marks have canonically equivalent glyphs
        // in the bundled fonts (U+037E → semicolon, U+0387 → middle dot).
        let display = match c {
            '\u{37e}' => ';',
            '\u{387}' => '\u{b7}',
            _ => c,
        };
        let galley = ui.fonts(|f| {
            f.layout_no_wrap(
                display.to_string(),
                FontId::new(height, family.clone()),
                foreground,
            )
        });
        let advance =
            cell_width.unwrap_or_else(|| galley.size().x * height / galley.size().y.max(1.0));
        glyphs.insert(
            c,
            Glyph {
                galley: styled_glyph(galley, advance, height, style),
                advance,
            },
        );
    }
    let lines = layout(
        paragraphs,
        rect.width(),
        object.options.auto_wrap,
        object.options.wrap_on_hyphen,
        object.justification.horizontal,
        |c| glyphs[&c].advance,
    );
    let block_height = lines.len() as f32 * height;
    let mut y = match object.justification.vertical {
        VerticalAlignment::Middle => rect.center().y - block_height / 2.0,
        VerticalAlignment::Bottom => rect.bottom() - block_height,
        _ => rect.top(),
    };
    for line in lines {
        let width: f32 = line.iter().map(|c| glyphs[c].advance).sum();
        let mut x = match object.justification.horizontal {
            HorizontalAlignment::Middle => rect.center().x - width / 2.0,
            HorizontalAlignment::Right => rect.right() - width,
            _ => rect.left(),
        };
        let left = x;
        if y + height > rect.top() && y < rect.bottom() {
            for c in line {
                let glyph = &glyphs[&c];
                if x + glyph.advance > rect.left() && x < rect.right() {
                    painter.galley(egui::pos2(x, y), glyph.galley.clone(), foreground);
                }
                x += glyph.advance;
            }
            for offset in [
                style.underlined.then_some(height - 1.0),
                style.crossed_out.then_some(height * 0.5),
            ]
            .into_iter()
            .flatten()
            {
                painter.line_segment(
                    [
                        egui::pos2(left, y + offset),
                        egui::pos2(left + width, y + offset),
                    ],
                    Stroke::new(1.0, foreground),
                );
            }
        }
        y += height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_line_breaks_and_wrap_opportunities() {
        let lines = |text, width, hyphen| {
            layout(
                controls(text),
                width,
                true,
                hyphen,
                HorizontalAlignment::Left,
                |_| 1.0,
            )
            .into_iter()
            .map(|line| line.into_iter().collect::<String>())
            .collect::<Vec<_>>()
        };
        assert_eq!(lines("ABC \rDEF", 4.0, false), ["ABC ", "DEF"]);
        assert_eq!(lines("ABC  \rDEF", 4.0, false), ["ABC ", "", "DEF"]);
        assert_eq!(
            lines("A\n\rB\r\nC\0ignored", 10.0, false),
            ["A", "", "B", "C"]
        );
        assert_eq!(lines("ab\u{ad}cd", 3.0, false), ["ab-", "cd"]);
        assert_eq!(lines("ab\u{ad}cd", 4.0, false), ["abcd"]);
        assert_eq!(lines("a-bcd", 4.0, true), ["a-", "bcd"]);
        assert_eq!(lines("a-bcd", 4.0, false), ["a-bc", "d"]);
        assert_eq!(lines("a\u{a0}bc", 3.0, false), ["a\u{a0}b", "c"]);
        assert_eq!(lines("A\r B", 4.0, false), ["A", " B"]);
    }

    #[test]
    fn painting_uses_declared_cells_and_intersected_clip() {
        use ag_iso_stack::object_pool::{ObjectId, ObjectType};
        let mut pool = ObjectPool::default();
        let mut font = crate::default_object(ObjectType::FontAttributes);
        let Object::FontAttributes(attributes) = &mut font else {
            unreachable!()
        };
        attributes.id = ObjectId::new(1).unwrap();
        attributes.font_size = FontSize::NonProportional(
            ag_iso_stack::object_pool::object_attributes::NonProportionalFontSize::Px8x12,
        );
        attributes.font_style.bold = true;
        attributes.font_style.italic = true;
        pool.add(font);
        let Object::OutputString(mut object) = crate::default_object(ObjectType::OutputString)
        else {
            unreachable!()
        };
        object.font_attributes = ObjectId::new(1).unwrap();
        object.width = 40;
        object.height = 24;
        object.value = "AB".into();
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = Rect::from_min_size(ui.max_rect().min, egui::vec2(20.0, 24.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.set_clip_rect(rect);
                    render(&object, ui, &pool, Point::default());
                });
            });
        });
        let glyphs: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some((shape.clip_rect, text))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(glyphs.len(), 2);
        for (clip, text) in &glyphs {
            assert_eq!(clip.width(), 20.0);
            assert_eq!(text.galley.size(), egui::vec2(8.0, 12.0));
            let cell = Rect::from_min_size(text.pos, egui::vec2(8.0, 12.0));
            assert!(cell
                .expand(0.001)
                .contains_rect(text.visual_bounding_rect()));
        }
        assert_eq!(glyphs[1].1.pos.x - glyphs[0].1.pos.x, 8.0);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(background) if background.rect.size() == egui::vec2(40.0, 24.0) && shape.clip_rect.width() == 20.0)));
    }
}
