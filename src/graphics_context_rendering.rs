//! Persistent pixel canvas used by Graphics Context objects in local preview.

use ag_iso_stack::object_pool::{object::GraphicsContext, ObjectPool};
use eframe::egui;

/// The retained, indexed-colour canvas for one Graphics Context object.
#[derive(Debug, Clone)]
pub struct GraphicsContextCanvas {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
    pub cursor: (i16, i16),
    pub foreground: u8,
    pub background: u8,
    pub viewport: (i16, i16, u16, u16),
    pub zoom: f32,
    pub transparent: bool,
    pub transparency_colour: u8,
    pub line_attributes: Option<u16>,
    pub fill_attributes: Option<u16>,
    pub font_attributes: Option<u16>,
    pub use_attribute_colours: bool,
}

impl GraphicsContextCanvas {
    pub fn update_attributes(&mut self, object: &GraphicsContext) {
        self.viewport = (
            object.viewport_x,
            object.viewport_y,
            object.viewport_width,
            object.viewport_height,
        );
        self.cursor = (object.graphics_cursor_x, object.graphics_cursor_y);
        self.zoom = object.viewport_zoom;
        self.foreground = object.foreground_colour;
        self.background = object.background_colour;
        self.line_attributes = object.line_attributes_object.0.map(|id| id.value());
        self.fill_attributes = object.fill_attributes_object.0.map(|id| id.value());
        self.font_attributes = object.font_attributes_object.0.map(|id| id.value());
        self.transparent = object.options.transparent;
        self.use_attribute_colours = bool::from(object.options.color);
        self.transparency_colour = object.transparency_colour;
    }

    pub fn write_attributes(&self, object: &mut GraphicsContext) {
        use ag_iso_stack::object_pool::NullableObjectId;
        object.viewport_x = self.viewport.0;
        object.viewport_y = self.viewport.1;
        object.viewport_width = self.viewport.2;
        object.viewport_height = self.viewport.3;
        object.graphics_cursor_x = self.cursor.0;
        object.graphics_cursor_y = self.cursor.1;
        object.viewport_zoom = self.zoom;
        object.foreground_colour = self.foreground;
        object.background_colour = self.background;
        object.line_attributes_object = self
            .line_attributes
            .map_or(NullableObjectId::NULL, NullableObjectId::new);
        object.fill_attributes_object = self
            .fill_attributes
            .map_or(NullableObjectId::NULL, NullableObjectId::new);
        object.font_attributes_object = self
            .font_attributes
            .map_or(NullableObjectId::NULL, NullableObjectId::new);
    }
    pub fn try_from_object(object: &GraphicsContext, max_pixels: usize) -> Result<Self, String> {
        if !object.viewport_zoom.is_finite()
            || object.viewport_zoom <= 0.0
            || object.viewport_zoom > 32.0
        {
            return Err("Graphics Context viewport zoom must be finite and in (0, 32]".into());
        }
        let count = usize::from(object.canvas_width)
            .checked_mul(usize::from(object.canvas_height))
            .filter(|count| *count <= max_pixels)
            .ok_or_else(|| {
                format!("Graphics Context canvas exceeds the {max_pixels} pixel preview limit")
            })?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| "unable to allocate Graphics Context canvas".to_owned())?;
        pixels.resize(count, object.background_colour);
        Ok(Self {
            width: object.canvas_width,
            height: object.canvas_height,
            pixels,
            cursor: (object.graphics_cursor_x, object.graphics_cursor_y),
            foreground: object.foreground_colour,
            background: object.background_colour,
            viewport: (
                object.viewport_x,
                object.viewport_y,
                object.viewport_width,
                object.viewport_height,
            ),
            zoom: object.viewport_zoom,
            transparent: object.options.transparent,
            transparency_colour: object.transparency_colour,
            line_attributes: object.line_attributes_object.0.map(|id| id.value()),
            fill_attributes: object.fill_attributes_object.0.map(|id| id.value()),
            font_attributes: object.font_attributes_object.0.map(|id| id.value()),
            use_attribute_colours: bool::from(object.options.color),
        })
    }

    pub fn clear(&mut self, colour: u8) {
        self.pixels.fill(colour);
        self.background = colour;
    }

    pub fn pixel(&mut self, x: i32, y: i32, colour: u8) {
        if x >= 0 && y >= 0 && x < i32::from(self.width) && y < i32::from(self.height) {
            self.pixels[y as usize * usize::from(self.width) + x as usize] = colour;
        }
    }

    pub fn line_to(&mut self, x: i16, y: i16, colour: u8) {
        let (mut x0, mut y0) = (i32::from(self.cursor.0), i32::from(self.cursor.1));
        let (x1, y1) = (i32::from(x), i32::from(y));
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut error = dx + dy;
        loop {
            self.pixel(x0, y0, colour);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                x0 += sx;
            }
            if twice <= dx {
                error += dx;
                y0 += sy;
            }
        }
        self.cursor = (x, y);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: u16, height: u16, colour: u8) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = x
            .saturating_add(i32::from(width))
            .min(i32::from(self.width));
        let y1 = y
            .saturating_add(i32::from(height))
            .min(i32::from(self.height));
        for py in y0..y1 {
            for px in x0..x1 {
                self.pixel(px, py, colour);
            }
        }
    }

    /// Convert the visible, zoomed viewport to an egui image. Canvas pixels
    /// outside the canvas are transparent, as the viewport is allowed to pan
    /// beyond its bounds.
    pub fn try_viewport_image(
        &self,
        pool: &ObjectPool,
        max_pixels: usize,
    ) -> Result<egui::ColorImage, String> {
        if !self.zoom.is_finite() || self.zoom <= 0.0 || self.zoom > 32.0 {
            return Err("Graphics Context viewport zoom must be finite and in (0, 32]".into());
        }
        let (_, _, width, height) = self.viewport;
        let w = usize::from(width);
        let h = usize::from(height);
        let count = w
            .checked_mul(h)
            .filter(|count| *count <= max_pixels)
            .ok_or_else(|| {
                format!("Graphics Context viewport exceeds the {max_pixels} pixel preview limit")
            })?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| "unable to allocate Graphics Context viewport image".to_owned())?;
        pixels.resize(count, egui::Color32::TRANSPARENT);
        let mut image = egui::ColorImage::new([w, h], pixels);
        let colours = crate::effective_colour::colours(pool);
        for y in 0..h {
            for x in 0..w {
                let source_x = i32::from(self.viewport.0) + (x as f32 / self.zoom) as i32;
                let source_y = i32::from(self.viewport.1) + (y as f32 / self.zoom) as i32;
                if source_x < 0
                    || source_y < 0
                    || source_x >= i32::from(self.width)
                    || source_y >= i32::from(self.height)
                {
                    continue;
                }
                let index =
                    self.pixels[source_y as usize * usize::from(self.width) + source_x as usize];
                if self.transparent && index == self.transparency_colour {
                    continue;
                }
                let colour = colours[index as usize];
                image[(x, y)] =
                    egui::Color32::from_rgba_unmultiplied(colour.r, colour.g, colour.b, colour.a);
            }
        }
        Ok(image)
    }
}

pub(crate) fn default_pixel_limit() -> usize {
    png::Limits::default().bytes / 4
}
