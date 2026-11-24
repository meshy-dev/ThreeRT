//! Acceptance check of the post chain (src/post.rs) against measured reference data:
//!
//!   cargo run --release --example post_check -- <dir> [--preset presets/cinematic.json] [--override JSON]
//!                                                  [--out DIR (the chain's pictures of the pairs)] [--json out.json]
//!
//! `<dir>` holds any of (what is absent is skipped; the data is not part of the repository):
//! - `tonemap-default.csv`: colours through colour grading and the tone curve at the preset's settings, rows
//!   `in_r,in_g,in_b,out_r,out_g,out_b` (exposed linear sRGB in, display-linear sRGB out), the first 257 a neutral
//!   ramp. Reported: the ramp's largest difference in sRGB-encoded value for inputs 1e-3 .. 16, and CIE dE2000 over
//!   all rows (mean, 99th percentile, largest).
//! - `grade-<setting>-<value>.csv`: the same with one grading setting changed (`whiteTemp-3000`, `whiteTint-m0.5`,
//!   `saturation-1.5`, `contrast-0.8`, `shadowsGain-0.7`, `highlightsGain-1.3`, `shadowsGain-rgb` = 1.2, 1, 0.8).
//! - `pairs/<scene>.hdr.f32` (width x height x 3 little-endian f32, scene-linear nits, rows from the top),
//!   `<scene>.png` (the picture it is to become) and `<scene>.stats.json` (`width`, `height`, `ev100`): `post::finish`
//!   of the first against the second, dE2000 mean and 99th percentile per scene.
//!
//!
//!   cargo run --release --example post_check -- --pictures a.png b.png
//!
//! compares two finished pictures instead (dE2000 mean, 99th percentile, largest; tools/post-live.ts).
//!
//! Exit status 1 if a target is missed (the constants below).

use glam::Vec3;
use std::path::{Path, PathBuf};
use threert_render::desc::{Preset, Rgb};
use threert_render::post;

/// Targets: neutral ramp (sRGB-encoded), colours (dE2000 mean, 99th percentile), each grading table (mean), each
/// pair (mean, 99th percentile).
const RAMP: f32 = 0.004;
const COLOURS: (f32, f32) = (1.0, 4.0);
const GRADE: f32 = 2.0;
const PAIR: (f32, f32) = (1.0, 3.0);

/// CIE 1976 L*a*b* of a display-linear sRGB colour (D65 white; IEC 61966-2-1 primaries, CIE 15).
fn lab(c: Vec3) -> [f32; 3] {
    let c = c.clamp(Vec3::ZERO, Vec3::ONE);
    let xyz = [0.4123908 * c.x + 0.3575843 * c.y + 0.1804808 * c.z, 0.2126390 * c.x + 0.7151687 * c.y + 0.0721923 * c.z, 0.0193308 * c.x + 0.1191948 * c.y + 0.9505322 * c.z];
    let white = [0.9504559, 1.0, 1.0890578];
    let f = |t: f32| if t > (6.0f32 / 29.0).powi(3) { t.cbrt() } else { t / (3.0 * (6.0f32 / 29.0).powi(2)) + 4.0 / 29.0 };
    let (fx, fy, fz) = (f(xyz[0] / white[0]), f(xyz[1] / white[1]), f(xyz[2] / white[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIE dE2000 (CIE 142-2001; Sharma, Wu & Dalal 2005, "The CIEDE2000 Color-Difference Formula: Implementation Notes,
/// Supplementary Test Data, and Mathematical Observations", Color Research & Application 30).
fn de2000(l1: [f32; 3], l2: [f32; 3]) -> f32 {
    let (c1, c2) = ((l1[1] * l1[1] + l1[2] * l1[2]).sqrt(), (l2[1] * l2[1] + l2[2] * l2[2]).sqrt());
    let cb = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cb.powi(7) / (cb.powi(7) + 25f32.powi(7))).sqrt());
    let (a1, a2) = ((1.0 + g) * l1[1], (1.0 + g) * l2[1]);
    let (c1p, c2p) = ((a1 * a1 + l1[2] * l1[2]).sqrt(), (a2 * a2 + l2[2] * l2[2]).sqrt());
    let hue = |b: f32, a: f32| if a == 0.0 && b == 0.0 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
    let (h1, h2) = (hue(l1[2], a1), hue(l2[2], a2));
    let (dl, dc) = (l2[0] - l1[0], c2p - c1p);
    let mut dh = h2 - h1;
    if c1p * c2p == 0.0 {
        dh = 0.0;
    } else if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    let dhp = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let (lb, cbp) = ((l1[0] + l2[0]) / 2.0, (c1p + c2p) / 2.0);
    let hs = h1 + h2;
    let hb = if c1p * c2p == 0.0 {
        hs
    } else if (h1 - h2).abs() > 180.0 {
        if hs < 360.0 { (hs + 360.0) / 2.0 } else { (hs - 360.0) / 2.0 }
    } else {
        hs / 2.0
    };
    let t = 1.0 - 0.17 * (hb - 30.0).to_radians().cos() + 0.24 * (2.0 * hb).to_radians().cos() + 0.32 * (3.0 * hb + 6.0).to_radians().cos() - 0.20 * (4.0 * hb - 63.0).to_radians().cos();
    let dth = 30.0 * (-((hb - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cbp.powi(7) / (cbp.powi(7) + 25f32.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lb - 50.0).powi(2) / (20.0 + (lb - 50.0).powi(2)).sqrt();
    let (sc, sh) = (1.0 + 0.045 * cbp, 1.0 + 0.015 * cbp * t);
    let rt = -(2.0 * dth).to_radians().sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhp / sh).powi(2) + rt * (dc / sc) * (dhp / sh)).max(0.0).sqrt()
}

fn srgb_decode(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Mean, 99th percentile and largest of a set of differences.
fn stats(mut d: Vec<f32>) -> (f32, f32, f32) {
    d.sort_by(|a, b| a.total_cmp(b));
    let mean = d.iter().sum::<f32>() / d.len().max(1) as f32;
    (mean, d[((d.len() as f32 * 0.99) as usize).min(d.len() - 1)], *d.last().unwrap())
}

fn table(path: &Path) -> anyhow::Result<Vec<[f32; 6]>> {
    let mut rows = vec![];
    for line in std::fs::read_to_string(path)?.lines().skip(1) {
        let v: Vec<f32> = line.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<_, _>>()?;
        anyhow::ensure!(v.len() == 6, "{}: a row of {} numbers", path.display(), v.len());
        rows.push([v[0], v[1], v[2], v[3], v[4], v[5]]);
    }
    Ok(rows)
}

/// The grading a table's name asks for, on `p`.
fn grading(name: &str, p: &mut threert_render::desc::PostPreset) -> anyhow::Result<()> {
    let (setting, value) = name.rsplit_once('-').ok_or_else(|| anyhow::anyhow!("{name}: no value"))?;
    let g = &mut p.color_grading;
    let rgb = if value == "rgb" { Rgb([1.2, 1.0, 0.8]) } else { Rgb([value.replace('m', "-").replace('p', "").parse()?; 3]) };
    match setting {
        "whiteTemp" => g.white_temp = rgb.0[0],
        "whiteTint" => g.white_tint = rgb.0[0],
        "saturation" => g.saturation = rgb,
        "contrast" => g.contrast = rgb,
        "shadowsGain" => g.shadows_gain = rgb,
        "highlightsGain" => g.highlights_gain = rgb,
        _ => anyhow::bail!("{name}: a setting this check does not know"),
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--pictures") {
        anyhow::ensure!(args.len() == 3, "post_check --pictures a.png b.png");
        let (a, b) = (image::open(&args[1])?.to_rgb8(), image::open(&args[2])?.to_rgb8());
        anyhow::ensure!(a.dimensions() == b.dimensions(), "{} and {} differ in size", args[1], args[2]);
        let dec = |v: &[u8]| lab(Vec3::new(srgb_decode(v[0] as f32 / 255.0), srgb_decode(v[1] as f32 / 255.0), srgb_decode(v[2] as f32 / 255.0)));
        let (mean, p99, max) = stats(a.as_raw().chunks_exact(3).zip(b.as_raw().chunks_exact(3)).map(|(x, y)| de2000(dec(x), dec(y))).collect());
        println!("{}", serde_json::json!({ "mean": mean, "p99": p99, "max": max }));
        return Ok(());
    }
    let flag = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned());
    let dir = PathBuf::from(args.first().filter(|a| !a.starts_with("--")).ok_or_else(|| anyhow::anyhow!("post_check <dir> [--preset file] [--json out.json]"))?);
    let preset_path = flag("--preset").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../presets/cinematic.json"));
    // (`--override`: JSON merged over the preset, as a scene's `overrides` are.)
    let mut preset_json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&preset_path)?)?;
    if let Some(o) = flag("--override") {
        threert_render::desc::merge_json(&mut preset_json, &serde_json::from_str(&o)?);
    }
    let preset: Preset = serde_json::from_value(preset_json)?;
    let mut report = serde_json::Map::new();
    let mut missed = vec![];

    // Colour grading and the tone curve.
    let default = dir.join("tonemap-default.csv");
    if default.exists() {
        let rows = table(&default)?;
        let tm = post::Tonemapper::new(&preset.post);
        let mut ramp = 0f32;
        for r in rows.iter().take(257).filter(|r| r[0] == r[1] && r[1] == r[2] && r[0] >= 1e-3 && r[0] <= 16.0) {
            let d = tm.apply(Vec3::new(r[0], r[1], r[2]));
            for k in 0..3 {
                ramp = ramp.max((post::linear_to_srgb(d[k].clamp(0.0, 1.0)) - post::linear_to_srgb(r[3 + k].clamp(0.0, 1.0))).abs());
            }
        }
        let (mean, p99, max) = stats(rows.iter().map(|r| de2000(lab(tm.apply(Vec3::new(r[0], r[1], r[2]))), lab(Vec3::new(r[3], r[4], r[5])))).collect());
        println!("tone curve + grading, {} colours: neutral ramp max |d| {ramp:.5} (target {RAMP}); dE2000 mean {mean:.3} (target {}), p99 {p99:.2} (target {}), max {max:.2}", rows.len(), COLOURS.0, COLOURS.1);
        report.insert("tonemap-default".into(), serde_json::json!({ "ramp": ramp, "mean": mean, "p99": p99, "max": max }));
        if ramp > RAMP || mean > COLOURS.0 || p99 > COLOURS.1 {
            missed.push("tonemap-default".to_string());
        }
    }
    let mut grades: Vec<PathBuf> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("grade-") && n.ends_with(".csv"))).collect();
    grades.sort();
    for path in grades {
        let name = path.file_stem().unwrap().to_str().unwrap().trim_start_matches("grade-").to_string();
        let mut p = preset.post.clone();
        grading(&name, &mut p)?;
        let tm = post::Tonemapper::new(&p);
        let (mean, p99, max) = stats(table(&path)?.iter().map(|r| de2000(lab(tm.apply(Vec3::new(r[0], r[1], r[2]))), lab(Vec3::new(r[3], r[4], r[5])))).collect());
        println!("grade {name:20} dE2000 mean {mean:.3} (target {GRADE}), p99 {p99:.2}, max {max:.2}");
        report.insert(format!("grade-{name}"), serde_json::json!({ "mean": mean, "p99": p99, "max": max }));
        if mean > GRADE {
            missed.push(format!("grade-{name}"));
        }
    }

    // The whole chain on the reference pairs.
    let pairs = dir.join("pairs");
    if pairs.is_dir() {
        let mut scenes: Vec<String> = std::fs::read_dir(&pairs)?.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".hdr.f32").map(String::from))).collect();
        scenes.sort();
        for scene in scenes {
            let stats_json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(pairs.join(format!("{scene}.stats.json")))?)?;
            let n = |k: &str| stats_json[k].as_f64().ok_or_else(|| anyhow::anyhow!("{scene}.stats.json: no `{k}`"));
            let (w, h, ev100) = (n("width")? as usize, n("height")? as usize, n("ev100")? as f32);
            let raw = std::fs::read(pairs.join(format!("{scene}.hdr.f32")))?;
            anyhow::ensure!(raw.len() == w * h * 12, "{scene}.hdr.f32: {} bytes for {w} x {h}", raw.len());
            let px: Vec<Vec3> = raw.chunks_exact(12).map(|c| Vec3::new(f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap()), f32::from_le_bytes(c[8..12].try_into().unwrap()))).collect();
            let want = image::open(pairs.join(format!("{scene}.png")))?.to_rgb8();
            anyhow::ensure!(want.width() as usize == w && want.height() as usize == h, "{scene}.png is not {w} x {h}");
            let got = post::finish(&post::Image { w, h, px }, 2f32.powf(-ev100), 1.0, &preset.post);
            if let Some(out) = flag("--out") {
                std::fs::create_dir_all(&out)?;
                image::save_buffer(Path::new(&out).join(format!("{scene}.png")), &got, w as u32, h as u32, image::ExtendedColorType::Rgba8)?;
            }
            let dec = |v: &[u8]| lab(Vec3::new(srgb_decode(v[0] as f32 / 255.0), srgb_decode(v[1] as f32 / 255.0), srgb_decode(v[2] as f32 / 255.0)));
            let (mean, p99, max) = stats(got.chunks_exact(4).zip(want.as_raw().chunks_exact(3)).map(|(a, b)| de2000(dec(a), dec(b))).collect());
            println!("pair {scene:18} EV100 {ev100:5.1}: dE2000 mean {mean:.3} (target {}), p99 {p99:.2} (target {}), max {max:.2}", PAIR.0, PAIR.1);
            report.insert(format!("pair-{scene}"), serde_json::json!({ "mean": mean, "p99": p99, "max": max }));
            if mean > PAIR.0 || p99 > PAIR.1 {
                missed.push(format!("pair-{scene}"));
            }
        }
    }
    if let Some(out) = flag("--json") {
        std::fs::write(out, serde_json::to_string_pretty(&report)?)?;
    }
    if !missed.is_empty() {
        println!("missed: {}", missed.join(", "));
        std::process::exit(1);
    }
    Ok(())
}
