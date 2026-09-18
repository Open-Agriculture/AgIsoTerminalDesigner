//! Small deterministic raster recipes on a normalized 24-unit grid. No image model.
use ag_iso_stack::object_pool::ObjectPool;
use serde_json::{json, Value};
pub const ICONS: [&str; 13] = [
    "wifi",
    "lock",
    "unlock",
    "check",
    "warning",
    "info",
    "disconnect",
    "signal_low",
    "signal_medium",
    "signal_high",
    "refresh",
    "search",
    "settings",
];
fn line(x: f64, y: f64, a: (f64, f64), b: (f64, f64)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
    (x - a.0 - t * dx).hypot(y - a.1 - t * dy) < 1.15
}
fn pixel(icon: &str, x: f64, y: f64) -> bool {
    let radius = (x - 12.).hypot(y - 18.);
    match icon {
        "wifi" => {
            ((radius - 13.).abs() < 1.1 || (radius - 8.).abs() < 1.1)
                && y < 16.
                && y < 18. - (x - 12.).abs() * 0.5
                || radius < 1.8
        }
        "lock" | "unlock" => {
            ((6. ..=18.).contains(&x) && (11. ..=21.).contains(&y))
                || (((x - 12.).hypot(y - 10.) - 5.).abs() < 1.1
                    && y <= 11.
                    && (icon == "lock" || x < 14.))
        }
        "check" => line(x, y, (4., 12.), (10., 18.)) || line(x, y, (10., 18.), (21., 5.)),
        "warning" => {
            line(x, y, (12., 3.), (2., 21.))
                || line(x, y, (2., 21.), (22., 21.))
                || line(x, y, (22., 21.), (12., 3.))
                || line(x, y, (12., 9.), (12., 14.))
                || (x - 12.).hypot(y - 18.) < 1.
        }
        "info" => {
            ((x - 12.).hypot(y - 12.) - 10.).abs() < 1.
                || line(x, y, (12., 11.), (12., 18.))
                || (x - 12.).hypot(y - 7.) < 1.4
        }
        "disconnect" => line(x, y, (5., 5.), (19., 19.)) || line(x, y, (5., 19.), (19., 5.)),
        "signal_low" | "signal_medium" | "signal_high" => {
            (3. ..=7.).contains(&x) && (15. ..=21.).contains(&y)
                || icon != "signal_low" && (10. ..=14.).contains(&x) && (9. ..=21.).contains(&y)
                || icon == "signal_high" && (17. ..=21.).contains(&x) && (3. ..=21.).contains(&y)
        }
        "refresh" => {
            (((x - 12.).hypot(y - 12.) - 8.).abs() < 1.2 && !(x > 14. && y < 9.))
                || line(x, y, (20., 5.), (20., 11.))
                || line(x, y, (20., 11.), (14., 9.))
        }
        "search" => {
            ((x - 10.).hypot(y - 10.) - 6.).abs() < 1.2 || line(x, y, (15., 15.), (21., 21.))
        }
        "settings" => {
            let r = (x - 12.).hypot(y - 12.);
            let angle = (y - 12.).atan2(x - 12.);
            (r - 4.).abs() < 1. || (7. ..=9. + (angle * 8.).cos().max(0.) * 2.).contains(&r)
        }
        _ => false,
    }
}
pub fn compile(pool: &ObjectPool, d: &Value) -> Result<Vec<Value>, Value> {
    let icon = d["icon"].as_str().unwrap_or("");
    let dimension = |k: &str| d[k].as_u64().filter(|v| (8..=128).contains(v));
    let (Some(w), Some(h)) = (dimension("width"), dimension("height")) else {
        return Err(json!({"code":"AI_ICON_INVALID","message":"width/height must be 8..128"}));
    };
    if !ICONS.contains(&icon) || d["name"].as_str().is_none_or(str::is_empty) {
        return Err(json!({"code":"AI_ICON_INVALID","available":ICONS}));
    }
    let foreground = super::visual_intent::resolve(pool, &d["foreground"])?;
    let transparent = d["transparent"].as_bool().unwrap_or(true);
    let background = if transparent {
        (0..=255)
            .find(|i| {
                crate::effective_colour::colour(pool, *i).as_rgb()
                    != crate::effective_colour::colour(pool, foreground.index).as_rgb()
            })
            .ok_or_else(|| json!({"code":"AI_ICON_NO_TRANSPARENCY_KEY"}))?
    } else {
        super::visual_intent::resolve(pool, &d["background"])?.index
    };
    let data: Vec<u8> = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                if pixel(
                    icon,
                    (x as f64 + 0.5) * 24. / w as f64,
                    (y as f64 + 0.5) * 24. / h as f64,
                ) {
                    foreground.index
                } else {
                    background
                }
            })
        })
        .collect();
    let target = json!({"name":d["name"]});
    let mut ops = vec![
        json!({"type":"CreateObject","data":{"object_type":"PictureGraphic","name":d["name"],"object_id":d["object_id"]}}),
    ];
    for (property, value) in [
        ("width", json!(w)),
        ("actual_width", json!(w)),
        ("actual_height", json!(h)),
        ("format", json!("EightBit")),
        (
            "options",
            json!({"transparent":transparent,"flashing":false,"data_code_type":"Raw"}),
        ),
        ("transparency_colour", json!(background)),
        ("data", json!(data)),
    ] {
        ops.push(json!({"type":"SetProperty","data":{"object":target,"property":property,"value":value}}));
    }
    if !d["parent"].is_null() {
        ops.push(json!({"type":"AddChild","data":{"parent":d["parent"],"child":target,"x":d["x"].as_i64().unwrap_or(0),"y":d["y"].as_i64().unwrap_or(0)}}));
    }
    Ok(ops)
}
