use crate::types::{CanvasRect, surface_matches_len};

pub fn canonicalize_fill_style(raw: &str) -> Option<String> {
    let value = raw.trim().to_ascii_lowercase();
    if value.starts_with('#') {
        return canonical_hex_color(&value);
    }
    if let Some((red, green, blue)) = css_named_color_rgb(&value) {
        return Some(format!("#{red:02x}{green:02x}{blue:02x}"));
    }
    if value == "transparent" {
        return Some("rgba(0, 0, 0, 0)".to_owned());
    }
    if let Some((red, green, blue, alpha)) = parse_rgb_function(&value) {
        if alpha == u8::MAX {
            return Some(format!("rgb({red}, {green}, {blue})"));
        }
        return Some(canonical_rgba(red, green, blue, alpha));
    }
    None
}

pub fn fill_style_rgba(style: &str) -> [u8; 4] {
    let value = style.trim().to_ascii_lowercase();
    if let Some(rgba) = hex_color_rgba(&value) {
        return rgba;
    }
    if let Some((red, green, blue, alpha)) = parse_rgb_function(&value) {
        return [red, green, blue, alpha];
    }
    if let Some((red, green, blue)) = css_named_color_rgb(&value) {
        return [red, green, blue, 255];
    }
    if value == "transparent" {
        return [0, 0, 0, 0];
    }
    [0, 0, 0, 255]
}

fn hex_color_rgba(value: &str) -> Option<[u8; 4]> {
    let hex = value.strip_prefix('#')?;
    if hex.is_empty() || !hex.chars().all(|char| char.is_ascii_hexdigit()) {
        return None;
    }
    let (red, green, blue, alpha) = match hex.len() {
        3 => (
            hex[0..1].repeat(2),
            hex[1..2].repeat(2),
            hex[2..3].repeat(2),
            "ff".to_owned(),
        ),
        4 => (
            hex[0..1].repeat(2),
            hex[1..2].repeat(2),
            hex[2..3].repeat(2),
            hex[3..4].repeat(2),
        ),
        6 => (
            hex[0..2].to_owned(),
            hex[2..4].to_owned(),
            hex[4..6].to_owned(),
            "ff".to_owned(),
        ),
        8 => (
            hex[0..2].to_owned(),
            hex[2..4].to_owned(),
            hex[4..6].to_owned(),
            hex[6..8].to_owned(),
        ),
        _ => return None,
    };
    Some([
        u8::from_str_radix(&red, 16).ok()?,
        u8::from_str_radix(&green, 16).ok()?,
        u8::from_str_radix(&blue, 16).ok()?,
        u8::from_str_radix(&alpha, 16).ok()?,
    ])
}

fn parse_rgb_function(value: &str) -> Option<(u8, u8, u8, u8)> {
    let body = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))?
        .strip_suffix(')')?;
    let parts = body.split(',').map(|part| part.trim()).collect::<Vec<_>>();
    let red = parse_channel(parts.first()?)?;
    let green = parse_channel(parts.get(1)?)?;
    let blue = parse_channel(parts.get(2)?)?;
    let alpha = match parts.get(3) {
        Some(alpha) => {
            let parsed = if let Some(percent) = alpha.strip_suffix('%') {
                percent.trim().parse::<f64>().ok()? / 100.0
            } else {
                alpha.trim().parse::<f64>().ok()?
            };
            (parsed.clamp(0.0, 1.0) * 255.0).round() as u8
        }
        None => 255,
    };
    Some((red, green, blue, alpha))
}

fn parse_channel(value: &str) -> Option<u8> {
    if let Some(percent) = value.strip_suffix('%') {
        let value = percent.trim().parse::<f64>().ok()?;
        return Some((value.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8);
    }
    let value = value.trim().parse::<f64>().ok()?;
    Some(value.clamp(0.0, 255.0) as u8)
}

fn css_named_color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    Some(match value.to_ascii_lowercase().as_str() {
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aqua" => (0, 255, 255),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "black" => (0, 0, 0),
        "blanchedalmond" => (255, 235, 205),
        "blue" => (0, 0, 255),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "cyan" => (0, 255, 255),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "fuchsia" => (255, 0, 255),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "gray" | "grey" => (128, 128, 128),
        "green" => (0, 128, 0),
        "greenyellow" => (173, 255, 47),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "magenta" => (255, 0, 255),
        "maroon" => (128, 0, 0),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "navy" => (0, 0, 128),
        "oldlace" => (253, 245, 230),
        "olive" => (128, 128, 0),
        "olivedrab" => (107, 142, 35),
        "orange" => (255, 165, 0),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "red" => (255, 0, 0),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "silver" => (192, 192, 192),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "teal" => (0, 128, 128),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "white" => (255, 255, 255),
        "whitesmoke" => (245, 245, 245),
        "yellow" => (255, 255, 0),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    })
}

pub fn normalize_rect(x: f64, y: f64, width: f64, height: f64) -> Option<CanvasRect> {
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
        return None;
    }
    let left = if width >= 0.0 { x } else { x + width };
    let top = if height >= 0.0 { y } else { y + height };
    let right = if width >= 0.0 { x + width } else { x };
    let bottom = if height >= 0.0 { y + height } else { y };
    Some((
        left.floor() as i32,
        top.floor() as i32,
        right.ceil() as i32,
        bottom.ceil() as i32,
    ))
}

pub fn paint_rect(
    pixels: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    rect: CanvasRect,
    rgba: [u8; 4],
) {
    if !surface_matches_len(pixels, canvas_width, canvas_height) {
        return;
    }
    let (left, top, right, bottom) = rect;
    if left >= right || top >= bottom {
        return;
    }
    let start_x = left.max(0).min(canvas_width as i32) as u32;
    let start_y = top.max(0).min(canvas_height as i32) as u32;
    let end_x = right.max(0).min(canvas_width as i32) as u32;
    let end_y = bottom.max(0).min(canvas_height as i32) as u32;
    for y in start_y..end_y {
        for x in start_x..end_x {
            let index = ((y * canvas_width + x) * 4) as usize;
            pixels[index..index + 4].copy_from_slice(&rgba);
        }
    }
}

fn canonical_hex_color(value: &str) -> Option<String> {
    let [red, green, blue, alpha] = hex_color_rgba(value)?;
    if alpha == u8::MAX {
        Some(format!("#{red:02x}{green:02x}{blue:02x}"))
    } else {
        Some(canonical_rgba(red, green, blue, alpha))
    }
}

fn canonical_rgba(red: u8, green: u8, blue: u8, alpha: u8) -> String {
    let fraction = f64::from(alpha) / 255.0;
    let rounded = (fraction * 100.0).round() / 100.0;
    if (rounded * 255.0).round() as u8 == alpha {
        format!("rgba({red}, {green}, {blue}, {rounded:.2})")
    } else {
        // Two decimal places cannot represent every 8-bit alpha. Retain enough
        // precision that serializing a style does not change its pixels.
        format!("rgba({red}, {green}, {blue}, {fraction:.3})")
    }
}
