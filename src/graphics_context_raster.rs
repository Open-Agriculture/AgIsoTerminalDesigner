//! Rasterize a normal object preview into the RGBA bitmap used by Graphics Context.

use std::collections::HashMap;

use ag_iso_stack::object_pool::{object::Object, vt_version::VtVersion, ObjectPool};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

pub(crate) fn rasterize(
    pool: &ObjectPool,
    object: &Object,
    width: u16,
    height: u16,
    version: VtVersion,
    soft_key_size: (u16, u16),
    max_pixels: usize,
) -> Result<egui::ColorImage, String> {
    rasterize_paint(width, height, max_pixels, |ui| {
        crate::object_rendering::render_object_with_settings(
            ui,
            pool,
            object,
            ag_iso_stack::object_pool::object_attributes::Point { x: 0, y: 0 },
            version,
            soft_key_size,
        );
    })
}

pub(crate) fn rasterize_paint(
    width: u16,
    height: u16,
    max_pixels: usize,
    mut paint: impl FnMut(&mut egui::Ui),
) -> Result<egui::ColorImage, String> {
    let size = [usize::from(width), usize::from(height)];
    if size.contains(&0) {
        return Err("Raster dimensions must be greater than zero".into());
    }
    let pixels = size[0]
        .checked_mul(size[1])
        .filter(|pixels| *pixels <= max_pixels)
        .ok_or_else(|| format!("Raster exceeds the {max_pixels} pixel limit"))?;

    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(width as f32, height as f32),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    paint(ui);
                });
        },
    );
    rasterize_output(&ctx, output, size, pixels)
}

fn rasterize_output(
    ctx: &egui::Context,
    output: egui::FullOutput,
    size: [usize; 2],
    pixel_count: usize,
) -> Result<egui::ColorImage, String> {
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let textures = apply_texture_deltas(output.textures_delta.set)?;
    rasterize_primitives(
        size,
        pixel_count,
        output.pixels_per_point,
        &primitives,
        &textures,
    )
}

fn apply_texture_deltas(
    deltas: Vec<(egui::TextureId, egui::epaint::ImageDelta)>,
) -> Result<HashMap<egui::TextureId, egui::ColorImage>, String> {
    let mut textures = HashMap::new();
    for (id, delta) in deltas {
        let image = match delta.image {
            egui::ImageData::Color(image) => image.as_ref().clone(),
        };
        if let Some([x, y]) = delta.pos {
            let target = textures
                .get_mut(&id)
                .ok_or_else(|| format!("Texture {id:?} received a patch before its full image"))?;
            patch_texture(target, &image, [x, y]);
        } else {
            textures.insert(id, image);
        }
    }
    Ok(textures)
}

fn patch_texture(target: &mut egui::ColorImage, patch: &egui::ColorImage, origin: [usize; 2]) {
    let [width, height] = target.size;
    for y in 0..patch.size[1].min(height.saturating_sub(origin[1])) {
        for x in 0..patch.size[0].min(width.saturating_sub(origin[0])) {
            target.pixels[(origin[1] + y) * width + origin[0] + x] =
                patch.pixels[y * patch.size[0] + x];
        }
    }
}

fn rasterize_primitives(
    size: [usize; 2],
    pixel_count: usize,
    pixels_per_point: f32,
    primitives: &[egui::ClippedPrimitive],
    textures: &HashMap<egui::TextureId, egui::ColorImage>,
) -> Result<egui::ColorImage, String> {
    let mut pixels = vec![Color32::TRANSPARENT; pixel_count];
    let screen_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0] as f32, size[1] as f32));
    for clipped in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &clipped.primitive else {
            continue;
        };
        let texture = textures.get(&mesh.texture_id);
        let clip = clipped.clip_rect.intersect(screen_rect);
        if clip.is_negative() {
            continue;
        }
        for triangle in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [
                mesh.vertices.get(triangle[0] as usize),
                mesh.vertices.get(triangle[1] as usize),
                mesh.vertices.get(triangle[2] as usize),
            ];
            let (Some(a), Some(b), Some(c)) = (a, b, c) else {
                continue;
            };
            rasterize_triangle(
                size,
                pixels_per_point,
                clip,
                [a, b, c],
                texture,
                &mut pixels,
            );
        }
    }
    Ok(egui::ColorImage::new(size, pixels))
}

fn rasterize_triangle(
    size: [usize; 2],
    pixels_per_point: f32,
    clip: Rect,
    vertices: [&egui::epaint::Vertex; 3],
    texture: Option<&egui::ColorImage>,
    pixels: &mut [Color32],
) {
    let mut vertices = vertices;
    let mut points = vertices.map(|vertex| vertex.pos * pixels_per_point);
    let mut area = edge(points[0], points[1], points[2]);
    if area.abs() < f32::EPSILON {
        return;
    }
    if area < 0.0 {
        vertices.swap(1, 2);
        points.swap(1, 2);
        area = -area;
    }
    let bounds = Rect::from_two_pos(points[0], points[1])
        .union(Rect::from_two_pos(points[1], points[2]))
        .union(Rect::from_two_pos(points[2], points[0]))
        .intersect(Rect::from_min_max(
            Pos2::new(clip.min.x * pixels_per_point, clip.min.y * pixels_per_point),
            Pos2::new(clip.max.x * pixels_per_point, clip.max.y * pixels_per_point),
        ));
    let x0 = (bounds.min.x.floor() as i32).clamp(0, size[0] as i32);
    let y0 = (bounds.min.y.floor() as i32).clamp(0, size[1] as i32);
    let x1 = (bounds.max.x.ceil() as i32).clamp(0, size[0] as i32);
    let y1 = (bounds.max.y.ceil() as i32).clamp(0, size[1] as i32);
    let colors = vertices.map(|vertex| vertex.color.to_array());

    for y in y0..y1 {
        for x in x0..x1 {
            let point = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
            let edges = [
                edge(points[1], points[2], point),
                edge(points[2], points[0], point),
                edge(points[0], points[1], point),
            ];
            if edges
                .iter()
                .zip([(1, 2), (2, 0), (0, 1)])
                .any(|(value, (start, end))| {
                    *value < -0.0001
                        || (value.abs() <= 0.0001 && !is_top_left(points[start], points[end]))
                })
            {
                continue;
            }
            let weights = edges.map(|edge| edge / area);
            let vertex_color: [u8; 4] = std::array::from_fn(|channel| {
                (0..3)
                    .map(|index| f32::from(colors[index][channel]) * weights[index])
                    .sum::<f32>()
                    .round()
                    .clamp(0.0, 255.0) as u8
            });
            let uv = vertices[0].uv.to_vec2() * weights[0]
                + vertices[1].uv.to_vec2() * weights[1]
                + vertices[2].uv.to_vec2() * weights[2];
            let sampled = texture.map_or([255; 4], |image| sample_texture(image, uv.to_pos2()));
            let source: [u8; 4] = std::array::from_fn(|channel| {
                ((u16::from(sampled[channel]) * u16::from(vertex_color[channel]) + 127) / 255) as u8
            });
            let index = y as usize * size[0] + x as usize;
            pixels[index] = source_over(pixels[index], source);
        }
    }
}

fn edge(a: Pos2, b: Pos2, point: Pos2) -> f32 {
    (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
}

fn is_top_left(a: Pos2, b: Pos2) -> bool {
    let delta = b - a;
    delta.y < 0.0 || (delta.y == 0.0 && delta.x > 0.0)
}

fn sample_texture(image: &egui::ColorImage, uv: Pos2) -> [u8; 4] {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return [0; 4];
    }
    let x = (uv.x * width as f32 - 0.5).clamp(0.0, width.saturating_sub(1) as f32);
    let y = (uv.y * height as f32 - 0.5).clamp(0.0, height.saturating_sub(1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let corners = [
        image.pixels[y0 * width + x0].to_array(),
        image.pixels[y0 * width + x1].to_array(),
        image.pixels[y1 * width + x0].to_array(),
        image.pixels[y1 * width + x1].to_array(),
    ];
    std::array::from_fn(|channel| {
        let top = f32::from(corners[0][channel]) * (1.0 - tx) + f32::from(corners[1][channel]) * tx;
        let bottom =
            f32::from(corners[2][channel]) * (1.0 - tx) + f32::from(corners[3][channel]) * tx;
        (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
    })
}

fn source_over(destination: Color32, source: [u8; 4]) -> Color32 {
    let destination = destination.to_array();
    let inverse_alpha = 255 - u16::from(source[3]);
    let output: [u8; 4] = std::array::from_fn(|channel| {
        (u16::from(source[channel]) + (u16::from(destination[channel]) * inverse_alpha + 127) / 255)
            .min(255) as u8
    });
    Color32::from_rgba_premultiplied(output[0], output[1], output[2], output[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::Mesh;

    fn quad_mesh(texture_id: egui::TextureId, color: Color32, uv: Rect) -> Mesh {
        let mut mesh = Mesh::with_texture(texture_id);
        mesh.add_rect_with_uv(Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)), uv, color);
        mesh
    }

    fn raster_mesh(mesh: Mesh, texture: egui::ColorImage) -> egui::ColorImage {
        let id = mesh.texture_id;
        let mut textures = HashMap::new();
        textures.insert(id, texture);
        rasterize_primitives(
            [4, 4],
            16,
            1.0,
            &[egui::ClippedPrimitive {
                clip_rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(4.0)),
                primitive: egui::epaint::Primitive::Mesh(mesh),
            }],
            &textures,
        )
        .unwrap()
    }

    #[test]
    fn solid_vertex_color_rasterizes_to_rgba_pixels() {
        let texture_id = egui::TextureId::Managed(0);
        let image = egui::ColorImage::filled([1, 1], Color32::WHITE);
        let raster = raster_mesh(
            quad_mesh(
                texture_id,
                Color32::from_rgb(200, 40, 10),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            ),
            image,
        );
        assert_eq!(raster.pixels[5], Color32::from_rgb(200, 40, 10));
        assert_eq!(raster.pixels[15], Color32::from_rgb(200, 40, 10));
    }

    #[test]
    fn texture_sampling_uses_the_mesh_uvs() {
        let texture_id = egui::TextureId::Managed(1);
        let texture = egui::ColorImage::new(
            [1, 1],
            vec![Color32::from_rgba_unmultiplied(20, 160, 70, 255)],
        );
        let raster = raster_mesh(
            quad_mesh(
                texture_id,
                Color32::WHITE,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            ),
            texture,
        );
        assert_eq!(raster.pixels[5], Color32::from_rgb(20, 160, 70));
    }

    #[test]
    fn adjacent_triangles_blend_shared_pixels_once() {
        let id = egui::TextureId::Managed(2);
        let mesh = quad_mesh(
            id,
            Color32::from_rgba_unmultiplied(255, 0, 0, 128),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        );
        let raster = raster_mesh(mesh, egui::ColorImage::filled([1, 1], Color32::WHITE));
        assert_eq!(raster.pixels[1 * 4 + 2].a(), 128);
        assert_eq!(raster.pixels[1 * 4 + 2].r(), 128);
    }

    #[test]
    fn offscreen_texture_and_font_atlas_are_sampled() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "raster-test-image",
            egui::ColorImage::filled([1, 1], Color32::from_rgb(15, 120, 220)),
            Default::default(),
        );
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(16.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.image((texture.id(), Vec2::splat(4.0)));
                        ui.painter().text(
                            Pos2::new(4.0, 0.0),
                            egui::Align2::LEFT_TOP,
                            "A",
                            egui::FontId::proportional(12.0),
                            Color32::WHITE,
                        );
                    });
            },
        );
        let raster = rasterize_output(&ctx, output, [16, 16], 256).unwrap();
        assert_eq!(raster.pixels[1 * 16 + 1], Color32::from_rgb(15, 120, 220));
        assert!(raster.pixels[4..].iter().any(|pixel| pixel.a() > 0));
    }
}
