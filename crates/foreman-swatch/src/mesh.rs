//! Indexed triangle mesh: weld, T-junction repair, watertight check, binary STL.
//!
//! `csgrs` returns loose convex polygons. Its `difference` only clips the
//! polygons near the cutter, so a clipped face can gain a vertex on an edge it
//! shares with an untouched neighbour (a T-junction). Welding and splitting
//! those edges restores a closed 2-manifold. This is mesh clean-up only; the
//! boolean itself is csgrs.

use std::{collections::HashMap, fmt};

use csgrs::mesh::Mesh;

/// Vertices closer than this are the same vertex, mm.
const WELD: f64 = 1e-6;
/// A vertex this close to an edge lies on it, mm.
const ON_EDGE: f64 = 1e-6;

#[derive(Debug, Clone, Default)]
pub struct IndexedMesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// Result of [`IndexedMesh::check`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshReport {
    pub triangles: usize,
    pub vertices: usize,
    /// Edges not shared by exactly two triangles.
    pub open_edges: usize,
    /// Directed edges used more than once (inconsistent winding / non-manifold).
    pub bad_orientation: usize,
    /// Signed volume, mm³ (positive when outward-facing).
    pub volume: f64,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl MeshReport {
    pub fn is_watertight(&self) -> bool {
        self.open_edges == 0 && self.bad_orientation == 0 && self.volume > 0.0
    }
}

impl fmt::Display for MeshReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} triangles, {} open edges, {} mis-oriented edges, volume {:.3} mm³",
            self.triangles, self.open_edges, self.bad_orientation, self.volume
        )
    }
}

type Cell = (i64, i64, i64);

struct Welder {
    map: HashMap<Cell, Vec<u32>>,
    vertices: Vec<[f64; 3]>,
}

impl Welder {
    fn cell(p: [f64; 3]) -> Cell {
        let c = |v: f64| (v / (WELD * 4.0)).floor() as i64;
        (c(p[0]), c(p[1]), c(p[2]))
    }
    fn index(&mut self, p: [f64; 3]) -> u32 {
        let (cx, cy, cz) = Self::cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = self.map.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &i in ids {
                            let q = self.vertices[i as usize];
                            if (q[0] - p[0]).abs() <= WELD
                                && (q[1] - p[1]).abs() <= WELD
                                && (q[2] - p[2]).abs() <= WELD
                            {
                                return i;
                            }
                        }
                    }
                }
            }
        }
        let i = self.vertices.len() as u32;
        self.vertices.push(p);
        self.map.entry((cx, cy, cz)).or_default().push(i);
        i
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl IndexedMesh {
    /// Fan-triangulates csgrs' convex polygons, welds vertices, drops
    /// degenerate triangles and splits T-junctions.
    pub fn from_csg<S: Clone + Send + Sync + fmt::Debug>(mesh: &Mesh<S>) -> Self {
        let mut w = Welder {
            map: HashMap::new(),
            vertices: Vec::new(),
        };
        let mut triangles = Vec::new();
        for poly in &mesh.polygons {
            let ids: Vec<u32> = poly
                .vertices
                .iter()
                .map(|v| w.index([v.pos.x, v.pos.y, v.pos.z]))
                .collect();
            for k in 1..ids.len().saturating_sub(1) {
                let t = [ids[0], ids[k], ids[k + 1]];
                if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                    triangles.push(t);
                }
            }
        }
        let mut m = IndexedMesh {
            vertices: w.vertices,
            triangles,
        };
        m.split_t_junctions();
        m.drop_degenerate();
        m
    }

    /// Directed edge → triangles using it.
    fn edge_uses(&self) -> HashMap<(u32, u32), Vec<usize>> {
        let mut e: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (ti, t) in self.triangles.iter().enumerate() {
            for k in 0..3 {
                e.entry((t[k], t[(k + 1) % 3])).or_default().push(ti);
            }
        }
        e
    }

    /// Every directed edge without its reverse is open. A vertex of another
    /// open edge that lies strictly inside it is a T-junction: split the
    /// triangle owning the edge there. Repeat until nothing changes.
    fn split_t_junctions(&mut self) {
        for _ in 0..64 {
            let uses = self.edge_uses();
            let open: Vec<(u32, u32)> = uses
                .keys()
                .filter(|&&(a, b)| !uses.contains_key(&(b, a)))
                .copied()
                .collect();
            if open.is_empty() {
                return;
            }
            let mut candidates: Vec<u32> = open.iter().flat_map(|&(a, b)| [a, b]).collect();
            candidates.sort_unstable();
            candidates.dedup();
            // triangle index → (edge slot, points on it ordered from start)
            let mut splits: HashMap<usize, (usize, Vec<u32>)> = HashMap::new();
            for &(a, b) in &open {
                let (pa, pb) = (self.vertices[a as usize], self.vertices[b as usize]);
                let d = sub(pb, pa);
                let len2 = dot(d, d);
                if len2 < 1e-24 {
                    continue;
                }
                let mut on: Vec<(f64, u32)> = candidates
                    .iter()
                    .filter(|&&v| v != a && v != b)
                    .filter_map(|&v| {
                        let p = self.vertices[v as usize];
                        let t = dot(sub(p, pa), d) / len2;
                        if t <= 1e-9 || t >= 1.0 - 1e-9 {
                            return None;
                        }
                        let c = cross(sub(p, pa), d);
                        let dist = (dot(c, c) / len2).sqrt();
                        (dist <= ON_EDGE).then_some((t, v))
                    })
                    .collect();
                if on.is_empty() {
                    continue;
                }
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                if let Some(&ti) = uses.get(&(a, b)).and_then(|v| v.first()) {
                    let t = self.triangles[ti];
                    let slot = (0..3).find(|&k| t[k] == a && t[(k + 1) % 3] == b);
                    if let Some(slot) = slot {
                        splits
                            .entry(ti)
                            .or_insert((slot, on.into_iter().map(|x| x.1).collect()));
                    }
                }
            }
            if splits.is_empty() {
                return;
            }
            let mut out = Vec::with_capacity(self.triangles.len() + splits.len() * 2);
            for (ti, t) in self.triangles.iter().enumerate() {
                match splits.get(&ti) {
                    None => out.push(*t),
                    Some((slot, pts)) => {
                        // Edge (a → b) gains pts; fan from the opposite corner c.
                        let a = t[*slot];
                        let b = t[(slot + 1) % 3];
                        let c = t[(slot + 2) % 3];
                        let mut chain = vec![a];
                        chain.extend(pts.iter().copied());
                        chain.push(b);
                        for w in chain.windows(2) {
                            out.push([w[0], w[1], c]);
                        }
                    }
                }
            }
            self.triangles = out;
        }
    }

    /// Drops triangles whose three corners coincide or that repeat a vertex.
    fn drop_degenerate(&mut self) {
        self.triangles
            .retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    }

    pub fn check(&self) -> MeshReport {
        let uses = self.edge_uses();
        let mut open = 0;
        let mut bad = 0;
        for (&(a, b), v) in &uses {
            if v.len() > 1 {
                bad += v.len() - 1;
            }
            match uses.get(&(b, a)) {
                Some(r) if r.len() == v.len() => {}
                _ => open += 1,
            }
        }
        let mut volume = 0.0;
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| self.vertices[i as usize]);
            volume += dot(a, cross(b, c)) / 6.0;
        }
        let mut min = [f64::MAX; 3];
        let mut max = [f64::MIN; 3];
        for p in &self.vertices {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        MeshReport {
            triangles: self.triangles.len(),
            vertices: self.vertices.len(),
            open_edges: open,
            bad_orientation: bad,
            volume,
            min,
            max,
        }
    }

    pub fn to_binary_stl(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(84 + self.triangles.len() * 50);
        let mut header = [b' '; 80];
        let name = b"Foreman swatch (Liberation Sans)";
        header[..name.len()].copy_from_slice(name);
        out.extend_from_slice(&header);
        out.extend_from_slice(&(self.triangles.len() as u32).to_le_bytes());
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| self.vertices[i as usize]);
            let n = cross(sub(b, a), sub(c, a));
            let l = dot(n, n).sqrt();
            let n = if l > 0.0 { n.map(|x| x / l) } else { [0.0; 3] };
            for v in [n, a, b, c] {
                for x in v {
                    out.extend_from_slice(&(x as f32).to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }

    /// Reads a binary STL (for tests and tools).
    pub fn from_binary_stl(data: &[u8]) -> Option<Self> {
        let n = u32::from_le_bytes(data.get(80..84)?.try_into().ok()?) as usize;
        let mut w = Welder {
            map: HashMap::new(),
            vertices: Vec::new(),
        };
        let mut triangles = Vec::with_capacity(n);
        for i in 0..n {
            let rec = data.get(84 + i * 50..84 + i * 50 + 50)?;
            let f = |k: usize| -> Option<f64> {
                Some(f64::from(f32::from_le_bytes(
                    rec.get(k..k + 4)?.try_into().ok()?,
                )))
            };
            let mut t = [0u32; 3];
            for (j, slot) in t.iter_mut().enumerate() {
                let o = 12 + j * 12;
                *slot = w.index([f(o)?, f(o + 4)?, f(o + 8)?]);
            }
            triangles.push(t);
        }
        Some(IndexedMesh {
            vertices: w.vertices,
            triangles,
        })
    }
}
