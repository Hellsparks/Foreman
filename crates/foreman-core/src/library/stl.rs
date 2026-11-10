//! STL thumbnail: a small pure-Rust software rasteriser (`stl_io` to read, `png` to encode).
//! Isometric view, z-buffer, flat shading, transparent background.

use std::{fs::File, io::BufReader, path::Path};

const BASE: [f32; 3] = [0.56, 0.62, 0.68];

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 {
        v
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

/// View transform: spin 45° about Z, then tilt so we look down at ~35°.
fn view(p: [f32; 3]) -> [f32; 3] {
    let (s, c) = (std::f32::consts::FRAC_PI_4).sin_cos();
    let x = p[0] * c - p[1] * s;
    let y = p[0] * s + p[1] * c;
    let (ts, tc) = 0.955_f32.sin_cos(); // ~54.7° from vertical
                                        // screen x, screen y (up), depth (towards viewer)
    [x, p[2] * ts + y * tc, -p[2] * tc + y * ts]
}

/// Render `path` to a `size`×`size` RGBA PNG.
pub fn render_png(path: &Path, size: u32) -> Result<Vec<u8>, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let mesh = stl_io::read_stl(&mut reader).map_err(|e| format!("stl: {e}"))?;
    if mesh.faces.is_empty() {
        return Err("stl: no triangles".into());
    }
    let verts: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| view(v.0)).collect();

    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for v in &verts {
        x0 = x0.min(v[0]);
        x1 = x1.max(v[0]);
        y0 = y0.min(v[1]);
        y1 = y1.max(v[1]);
    }
    let span = (x1 - x0).max(y1 - y0).max(f32::EPSILON);
    let n = size as usize;
    let margin = 0.08 * size as f32;
    let scale = (size as f32 - 2.0 * margin) / span;
    let off_x = margin + ((size as f32 - 2.0 * margin) - (x1 - x0) * scale) / 2.0;
    let off_y = margin + ((size as f32 - 2.0 * margin) - (y1 - y0) * scale) / 2.0;
    let to_px = |v: [f32; 3]| -> [f32; 3] {
        [
            off_x + (v[0] - x0) * scale,
            size as f32 - (off_y + (v[1] - y0) * scale),
            v[2],
        ]
    };

    let light = normalize([-0.4, 0.6, 0.7]);
    let mut depth = vec![f32::MIN; n * n];
    let mut rgba = vec![0u8; n * n * 4];
    for face in &mesh.faces {
        let [a, b, c] = face.vertices.map(|i| verts[i]);
        let normal = normalize(cross(sub(b, a), sub(c, a)));
        // Two-sided lighting so badly wound meshes still render.
        let diffuse = (normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2]).abs();
        let shade = 0.35 + 0.65 * diffuse;
        let px = [to_px(a), to_px(b), to_px(c)];
        let min_x = px
            .iter()
            .map(|p| p[0])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_x = px
            .iter()
            .map(|p| p[0])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(size as f32 - 1.0) as usize;
        let min_y = px
            .iter()
            .map(|p| p[1])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_y = px
            .iter()
            .map(|p| p[1])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(size as f32 - 1.0) as usize;
        let area = (px[1][0] - px[0][0]) * (px[2][1] - px[0][1])
            - (px[2][0] - px[0][0]) * (px[1][1] - px[0][1]);
        if area.abs() < f32::EPSILON {
            continue;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 =
                    ((px[1][0] - fx) * (px[2][1] - fy) - (px[2][0] - fx) * (px[1][1] - fy)) / area;
                let w1 =
                    ((px[2][0] - fx) * (px[0][1] - fy) - (px[0][0] - fx) * (px[2][1] - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * px[0][2] + w1 * px[1][2] + w2 * px[2][2];
                let i = y * n + x;
                if z > depth[i] {
                    depth[i] = z;
                    for (ch, base) in BASE.iter().enumerate() {
                        rgba[i * 4 + ch] = (base * shade * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                    rgba[i * 4 + 3] = 255;
                }
            }
        }
    }

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn renders_a_cube() {
        let dir = tempfile::TempDir::new().expect("dir");
        let path = dir.path().join("cube.stl");
        let v = |x: f32, y: f32, z: f32| [x, y, z];
        let quads = [
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.)],
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            [v(0., 1., 0.), v(1., 1., 0.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(0., 1., 0.), v(0., 1., 1.), v(0., 0., 1.)],
            [v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.), v(1., 0., 1.)],
        ];
        let mut s = String::from("solid cube\n");
        for q in quads {
            for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                s.push_str(" facet normal 0 0 0\n  outer loop\n");
                for p in tri {
                    s.push_str(&format!("   vertex {} {} {}\n", p[0], p[1], p[2]));
                }
                s.push_str("  endloop\n endfacet\n");
            }
        }
        s.push_str("endsolid cube\n");
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(s.as_bytes()))
            .expect("write");
        let png = render_png(&path, 64).expect("render");
        assert!(png.starts_with(b"\x89PNG"));
        assert!(png.len() > 100);
    }
}
