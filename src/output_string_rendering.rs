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

/// Whether the current value of a fixed-cell Output String exceeds its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextClipping {
    pub horizontal: bool,
    pub vertical: bool,
}

/// Measure an Output String using the same control handling and line layout as
/// the preview. Proportional fonts are intentionally omitted because their
/// terminal-specific glyph widths cannot be inferred reliably from the editor.
pub(crate) fn clipping(object: &OutputString, pool: &ObjectPool) -> Option<TextClipping> {
    let Object::FontAttributes(font) = pool.object_by_nullable_id(object.font_attributes)? else {
        return None;
    };
    let FontSize::NonProportional(font_size) = font.font_size else {
        return None;
    };
    if object.justification.horizontal == HorizontalAlignment::Reserved
        || object.justification.vertical == VerticalAlignment::Reserved
    {
        return None;
    }
    let value = if let Some(id) = object.variable_reference.0 {
        let Object::StringVariable(variable) = pool.object_by_id(id)? else {
            return None;
        };
        &variable.value
    } else {
        &object.value
    };
    let text = value.decode(font.font_type).ok()?;
    // NUL terminates presentation text; a string beginning with it has no
    // visible content to clip.
    if text.is_empty() || text.starts_with('\0') {
        return None;
    }

    let height = font_size.height() as f32;
    let cell_width = font_size.width() as f32;
    let paragraphs = controls(&text);
    let lines = layout(
        paragraphs,
        object.width as f32,
        object.options.auto_wrap,
        object.options.wrap_on_hyphen,
        object.justification.horizontal,
        |_| cell_width,
    );
    let horizontal = lines
        .iter()
        .any(|line| line.len() as f32 * cell_width > object.width as f32);
    let vertical = lines.len() as f32 * height > object.height as f32;
    Some(TextClipping {
        horizontal,
        vertical,
    })
}

/// Logical text width uses the same character cells as the preview renderer.
pub(crate) fn text_width(
    ui: &egui::Ui,
    font: &ag_iso_stack::object_pool::object::FontAttributes,
    text: &str,
) -> f32 {
    match font.font_size {
        FontSize::NonProportional(size) => text.chars().count() as f32 * size.width() as f32,
        FontSize::Proportional(height) => text
            .chars()
            .map(|c| {
                let galley = ui.fonts(|f| {
                    f.layout_no_wrap(
                        c.to_string(),
                        FontId::proportional(height as f32),
                        Color32::WHITE,
                    )
                });
                galley.size().x * height as f32 / galley.size().y.max(1.0)
            })
            .sum(),
    }
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
    vt_version: ag_iso_stack::object_pool::vt_version::VtVersion,
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
    let Some(Object::FontAttributes(font)) = pool.object_by_nullable_id(object.font_attributes)
    else {
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
        // Rasterize at the final visual scale, then fit the mesh back into its
        // unchanged logical character cell. egui supplies device DPI itself.
        let scale = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(egui::Id::new("vt_render_scale")))
            .unwrap_or(1.0)
            .clamp(1.0, 8.0);
        let galley = if scale > 1.0 {
            ui.fonts(|f| {
                f.layout_no_wrap(
                    display.to_string(),
                    FontId::new(height * scale, family.clone()),
                    foreground,
                )
            })
        } else {
            galley
        };
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
    let vertical = if vt_version >= ag_iso_stack::object_pool::vt_version::VtVersion::Version4 {
        object.justification.vertical
    } else {
        // Vertical justification was introduced in VT4. Top alignment is a
        // stable convention for previews targeting earlier terminals.
        VerticalAlignment::Top
    };
    let mut y = match vertical {
        VerticalAlignment::Middle => rect.center().y - block_height / 2.0,
        VerticalAlignment::Bottom => rect.bottom() - block_height,
        _ => rect.top(),
    };
    let legacy_cell = if vt_version < ag_iso_stack::object_pool::vt_version::VtVersion::Version4 {
        // Legacy justification is character based. For proportional fonts,
        // use a stable half-em cell as a deterministic preview convention.
        Some(cell_width.unwrap_or(height * 0.5).max(1.0))
    } else {
        None
    };
    for line in lines {
        let width: f32 = line.iter().map(|c| glyphs[c].advance).sum();
        let align_width = legacy_cell.map_or(width, |cell| line.len() as f32 * cell);
        let mut x = match object.justification.horizontal {
            HorizontalAlignment::Middle => rect.center().x - align_width / 2.0,
            HorizontalAlignment::Right => rect.right() - align_width,
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
