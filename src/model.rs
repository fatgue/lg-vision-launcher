use serde_json::Value;

#[derive(Clone, Debug)]
pub struct App {
    pub id: String,
    pub title: String,
    pub icon_path: Option<String>,
    /// Background colour LG declares for the icon, if any.
    pub bg_color: Option<String>,
    pub params: Option<Value>,
    /// Large 16:9 artwork for full-bleed tiles: the launch splash (logo on brand colour)
    /// when the app has one, else LG's launcher background image.
    pub art_path: Option<String>,
    pub art_is_splash: bool,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub id: String,
    pub label: String,
    pub app_id: String,
    pub connected: bool,
}

/// RGBA8 image (an icon ≤128 px or 320×180 tile artwork) with its dominant colour.
pub struct Icon {
    pub app_id: String,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    pub color: Option<[u8; 3]>,
}

impl Icon {
    /// The icon's own background: the mean border colour when the border is opaque and
    /// uniform. A tile filled with it makes the icon blend into a full-bleed logo tile.
    pub fn edge_color(&self) -> Option<[u8; 3]> {
        let (w, h) = (self.w as usize, self.h as usize);
        if w < 8 || h < 8 {
            return None;
        }
        let mut samples = Vec::new();
        for i in 0..w.max(h) {
            for (x, y) in [(i.min(w - 1), 1), (i.min(w - 1), h - 2), (1, i.min(h - 1)), (w - 2, i.min(h - 1))] {
                let p = &self.pixels[(y * w + x) * 4..][..4];
                if p[3] < 250 {
                    return None;
                }
                samples.push([p[0] as f32, p[1] as f32, p[2] as f32]);
            }
        }
        let n = samples.len() as f32;
        let mean = samples.iter().fold([0.0; 3], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]);
        let spread = samples.iter().map(|p| (p[0] - mean[0]).abs() + (p[1] - mean[1]).abs() + (p[2] - mean[2]).abs()).sum::<f32>() / n;
        (spread < 18.0).then(|| [mean[0] as u8, mean[1] as u8, mean[2] as u8])
    }
}

/// Messages for the user; the UI picks the localised text.
pub enum Notice {
    ServiceUnavailable,
    AppListUnavailable,
    LaunchFailed,
    #[cfg_attr(all(target_os = "linux", target_arch = "arm"), allow(dead_code))]
    PreviewLaunch(String),
}

pub enum Update {
    Apps(Vec<App>),
    Inputs(Vec<Input>),
    Icon(Icon),
    Art(Icon),
    Notice(Notice),
}

pub enum Request {
    Refresh,
    Launch { app_id: String, params: Option<Value> },
    SavePrefs(crate::storage::Prefs),
}

/// Normalises `listLaunchPoints` (LG's Home bar, already in the user's order), taking each
/// app's launch splash from `listApps` when available.
pub fn apps_from_launch_points(reply: &Value, apps: Option<&Value>, own_id: &str) -> Vec<App> {
    let Some(points) = reply.get("launchPoints").and_then(Value::as_array) else {
        return Vec::new();
    };
    let splashes: std::collections::HashMap<&str, String> = apps
        .and_then(|apps| apps.get("apps"))
        .and_then(Value::as_array)
        .map(|apps| {
            apps.iter()
                .filter_map(|app| {
                    let id = app.get("id")?.as_str()?;
                    let folder = app.get("folderPath")?.as_str()?;
                    let splash = app.get("splashBackground")?.as_str()?;
                    let path = if splash.starts_with('/') { splash.to_string() } else { format!("{folder}/{splash}") };
                    path.to_ascii_lowercase().ends_with(".png").then(|| (id, path))
                })
                .collect()
        })
        .unwrap_or_default();
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
            let splash = splashes.get(id.as_str()).cloned();
            let background = point
                .get("bgImage")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with('/') && path.to_ascii_lowercase().ends_with(".png"))
                .map(str::to_string);
            Some(App {
                art_is_splash: splash.is_some(),
                art_path: splash.or(background),
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
    let (rgba, w, h) = decode_png(bytes)?;
    let (pixels, w, h) = fit(&rgba, w, h, 128);
    Ok(Icon {
        app_id: app_id.to_string(),
        w: w as u32,
        h: h as u32,
        color: dominant_color(&pixels),
        pixels,
    })
}

pub const ART_W: usize = 320;
pub const ART_H: usize = 180;

/// Decodes 16:9 artwork into a 320×180 tile image. Splash screens are a logo on a plain
/// background: the logo is located and the crop chosen so it fills the tile. A splash
/// without a detectable logo is rejected (the tile then uses the app icon).
pub fn decode_art(app_id: &str, bytes: &[u8], splash: bool) -> Result<Icon, String> {
    let (rgba, w, h) = decode_png(bytes)?;
    let (x0, y0, cw, ch) = if splash { logo_crop(&rgba, w, h)? } else { cover_crop(w, h) };
    let pixels = resample(&rgba, w, (x0, y0, cw, ch), ART_W, ART_H);
    Ok(Icon {
        app_id: app_id.to_string(),
        w: ART_W as u32,
        h: ART_H as u32,
        color: dominant_color(&pixels),
        pixels,
    })
}

/// Largest centred 16:9 region.
fn cover_crop(w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (cw, ch) = if w * 9 >= h * 16 { (h * 16 / 9, h) } else { (w, w * 9 / 16) };
    ((w - cw) / 2, (h - ch) / 2, cw, ch)
}

/// Finds the logo (pixels clearly unlike the corner background) and frames it so it spans
/// about 58 % of the tile width or 52 % of its height.
fn logo_crop(rgba: &[u8], w: usize, h: usize) -> Result<(usize, usize, usize, usize), String> {
    let pixel = |x: usize, y: usize| &rgba[(y * w + x) * 4..][..4];
    let mut bg = [0f32; 3];
    for (cx, cy) in [(8, 8), (w - 9, 8), (8, h - 9), (w - 9, h - 9)] {
        let p = pixel(cx, cy);
        for c in 0..3 {
            bg[c] += p[c] as f32 / 4.0;
        }
    }
    let (mut x0, mut y0, mut x1, mut y1, mut hits) = (w, h, 0, 0, 0usize);
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let p = pixel(x, y);
            let diff = (0..3).map(|c| (p[c] as f32 - bg[c]).abs()).sum::<f32>() * p[3] as f32 / 255.0;
            if diff > 110.0 {
                hits += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if hits < 40 {
        return Err("splash has no detectable logo".into());
    }
    let (bw, bh) = ((x1 - x0 + 4) as f32, (y1 - y0 + 4) as f32);
    if bw > w as f32 * 0.85 && bh > h as f32 * 0.85 {
        return Ok(cover_crop(w, h));
    }
    // Never zoom past 1:1 (blur) nor beyond the image.
    let cw = (bw / 0.58).max(bh / 0.52 * 16.0 / 9.0).clamp(ART_W as f32 * 1.4, w as f32).min(h as f32 * 16.0 / 9.0);
    let ch = cw * 9.0 / 16.0;
    let cx = (x0 + x1) as f32 / 2.0;
    let cy = (y0 + y1) as f32 / 2.0;
    let left = (cx - cw / 2.0).clamp(0.0, w as f32 - cw);
    let top = (cy - ch / 2.0).clamp(0.0, h as f32 - ch);
    Ok((left as usize, top as usize, cw as usize, ch as usize))
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
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
    Ok((rgba, w, h))
}

/// Box-filter resample of a source region to nw×nh (premultiplied, so transparency is clean).
fn resample(rgba: &[u8], stride_w: usize, (sx, sy, sw, sh): (usize, usize, usize, usize), nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let (y0, y1) = (sy + y * sh / nh, (sy + (y + 1) * sh / nh).max(sy + y * sh / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (sx + x * sw / nw, (sx + (x + 1) * sw / nw).max(sx + x * sw / nw + 1));
            let mut sum = [0u32; 4];
            for py in y0..y1 {
                for px in x0..x1 {
                    let p = &rgba[(py * stride_w + px) * 4..][..4];
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
    out
}

/// Fits an image into max×max, keeping its aspect ratio.
fn fit(rgba: &[u8], w: usize, h: usize, max: usize) -> (Vec<u8>, usize, usize) {
    if w <= max && h <= max {
        return (rgba.to_vec(), w, h);
    }
    let scale = max as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    (resample(rgba, w, (0, 0, w, h), nw, nh), nw, nh)
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
