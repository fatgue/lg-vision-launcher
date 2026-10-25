use serde_json::Value;

#[derive(Clone, Debug)]
pub struct App {
    pub id: String,
    pub title: String,
    pub icon_path: Option<String>,
    /// Background colour LG declares for the icon, if any.
    pub bg_color: Option<String>,
    pub params: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub id: String,
    pub label: String,
    pub app_id: String,
    pub connected: bool,
}

/// RGBA8 icon, at most 128 px on its long side, with its dominant colour.
pub struct Icon {
    pub app_id: String,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    pub color: Option<[u8; 3]>,
}

/// Messages for the user; the UI picks the localised text.
pub enum Notice {
    ServiceUnavailable,
    AppListUnavailable,
    LaunchFailed,
    PreviewLaunch(String),
}

pub enum Update {
    Apps(Vec<App>),
    Inputs(Vec<Input>),
    Icon(Icon),
    Notice(Notice),
}

pub enum Request {
    Refresh,
    Launch { app_id: String, params: Option<Value> },
    SavePrefs(crate::storage::Prefs),
}

/// Normalises `listLaunchPoints` (LG's Home bar, already in the user's order).
pub fn apps_from_launch_points(reply: &Value, own_id: &str) -> Vec<App> {
    let Some(points) = reply.get("launchPoints").and_then(Value::as_array) else {
        return Vec::new();
    };
    points
        .iter()
        .filter_map(|point| {
            let id = point.get("id")?.as_str()?.to_string();
            if id == own_id || id == "com.webos.app.home" || id.starts_with("com.webos.app.hdmi") {
                return None;
            }
            let title = point.get("title").and_then(Value::as_str).unwrap_or(&id).trim().to_string();
            let icon_path = ["largeIcon", "icon"]
                .iter()
                .find_map(|key| point.get(*key).and_then(Value::as_str))
                .filter(|path| path.starts_with('/'))
                .map(str::to_string);
            Some(App {
                id,
                title,
                icon_path,
                bg_color: point.get("bgColor").and_then(Value::as_str).map(str::to_string),
                params: point.get("params").filter(|p| p.is_object()).cloned(),
            })
        })
        .collect()
}

/// Normalises EIM `getAllInputStatus`.
pub fn inputs_from_status(reply: &Value) -> Vec<Input> {
    let Some(devices) = reply.get("devices").and_then(Value::as_array) else {
        return Vec::new();
    };
    devices
        .iter()
        .filter_map(|device| {
            let id = device.get("id")?.as_str()?.to_string();
            let app_id = device.get("appId")?.as_str()?.to_string();
            let label = device
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(Input {
                id,
                label,
                app_id,
                connected: device.get("connected").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// Decodes a PNG, fits it into 128 px and finds its most saturated dominant colour.
pub fn decode_icon(app_id: &str, bytes: &[u8]) -> Result<Icon, String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buffer[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buffer[..w * h * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buffer[..w * h * 2].chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buffer[..w * h].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpanded palette".into()),
    };
    let (pixels, w, h) = fit(&rgba, w, h, 128);
    Ok(Icon {
        app_id: app_id.to_string(),
        w: w as u32,
        h: h as u32,
        color: dominant_color(&pixels),
        pixels,
    })
}

/// Box-filter downscale (premultiplied, so transparent edges do not darken).
fn fit(rgba: &[u8], w: usize, h: usize, max: usize) -> (Vec<u8>, usize, usize) {
    if w <= max && h <= max {
        return (rgba.to_vec(), w, h);
    }
    let scale = max as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &rgba[(sy * w + sx) * 4..][..4];
                    let a = p[3] as u32;
                    sum[0] += p[0] as u32 * a;
                    sum[1] += p[1] as u32 * a;
                    sum[2] += p[2] as u32 * a;
                    sum[3] += a;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = &mut out[(y * nw + x) * 4..][..4];
            if sum[3] > 0 {
                o[0] = (sum[0] / sum[3]) as u8;
                o[1] = (sum[1] / sum[3]) as u8;
                o[2] = (sum[2] / sum[3]) as u8;
            }
            o[3] = (sum[3] / n) as u8;
        }
    }
    (out, nw, nh)
}

fn dominant_color(rgba: &[u8]) -> Option<[u8; 3]> {
    let (mut acc, mut weight) = ([0f32; 3], 0f32);
    let (mut plain, mut plain_n) = ([0f32; 3], 0f32);
    for p in rgba.chunks_exact(4) {
        if p[3] < 128 {
            continue;
        }
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        plain = [plain[0] + r, plain[1] + g, plain[2] + b];
        plain_n += 1.0;
        if max < 0.12 {
            continue;
        }
        let saturation = (max - min) / max;
        let w = saturation * saturation * max;
        acc = [acc[0] + r * w, acc[1] + g * w, acc[2] + b * w];
        weight += w;
    }
    let pick = |c: [f32; 3], n: f32| [(c[0] / n * 255.0) as u8, (c[1] / n * 255.0) as u8, (c[2] / n * 255.0) as u8];
    if weight > 0.5 {
        Some(pick(acc, weight))
    } else if plain_n > 0.0 {
        Some(pick(plain, plain_n))
    } else {
        None
    }
}
