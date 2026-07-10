//! Ported from vtracer's `gradient::contour`.
//!
//! Pixel-exact **region contours**: every region of a labeling as closed crack-boundary
//! loops (outer contours + holes), for serialization as SVG paths. No curve fitting here —
//! vertices are integer pixel-corner coordinates with collinear runs merged, so the
//! polygons reproduce the label map exactly (fill-rule evenodd handles holes and
//! disconnected islands uniformly). Sub-pixel/coverage-informed fitting is a later stage.

use std::collections::HashMap;

/// One closed loop: pixel-corner vertices, collinear runs merged, implicitly closed.
pub type Loop = Vec<(u32, u32)>;

/// Trace all boundary loops of every label in one sweep. Returns `loops[label]` = the
/// region's closed loops (outer boundaries counter-clockwise in image coords, holes
/// clockwise — but callers should just use `fill-rule: evenodd`).
///
/// Edges follow the crack between pixels with the region interior on the LEFT of the
/// walking direction; at corner-touching vertices (four cracks meeting) the sharpest
/// left turn is taken, which keeps loops simple (non-self-crossing).
pub fn region_contours(labels: &[u32], w: usize, h: usize, count: usize) -> Vec<Vec<Loop>> {
    // directed crack edges per label; dir: 0=+x 1=+y 2=-x 3=-y, vertex grid is (w+1)x(h+1)
    let vw = (w + 1) as u32;
    let key = |x: u32, y: u32| (y * vw + x) as u64;
    // per label: map start-vertex -> outgoing (dir, end-vertex)
    let mut edges: Vec<HashMap<u64, Vec<(u8, u32, u32)>>> = vec![HashMap::new(); count];
    let mut push = |l: usize, sx: u32, sy: u32, dir: u8, ex: u32, ey: u32| {
        edges[l].entry(key(sx, sy)).or_default().push((dir, ex, ey));
    };
    for y in 0..h {
        for x in 0..w {
            let l = labels[y * w + x] as usize;
            let (xu, yu) = (x as u32, y as u32);
            let differs = |nx: i64, ny: i64| {
                nx < 0
                    || ny < 0
                    || nx >= w as i64
                    || ny >= h as i64
                    || labels[ny as usize * w + nx as usize] as usize != l
            };
            if differs(x as i64, y as i64 - 1) {
                push(l, xu, yu, 0, xu + 1, yu); // top crack, walking +x
            }
            if differs(x as i64 + 1, y as i64) {
                push(l, xu + 1, yu, 1, xu + 1, yu + 1); // right crack, walking +y
            }
            if differs(x as i64, y as i64 + 1) {
                push(l, xu + 1, yu + 1, 2, xu, yu + 1); // bottom crack, walking -x
            }
            if differs(x as i64 - 1, y as i64) {
                push(l, xu, yu + 1, 3, xu, yu); // left crack, walking -y
            }
        }
    }

    let mut out: Vec<Vec<Loop>> = Vec::with_capacity(count);
    for l in 0..count {
        let mut loops = Vec::new();
        let em = &mut edges[l];
        // walk until all edges consumed
        let starts: Vec<u64> = em.keys().copied().collect();
        for s in starts {
            loop {
                let Some(list) = em.get_mut(&s) else { break };
                let Some(&(d0, ex0, ey0)) = list.last() else {
                    em.remove(&s);
                    break;
                };
                list.pop();
                if list.is_empty() {
                    em.remove(&s);
                }
                // walk one loop
                let (sx, sy) = ((s % vw as u64) as u32, (s / vw as u64) as u32);
                let mut pts: Loop = vec![(sx, sy)];
                let (mut cx, mut cy, mut cd) = (ex0, ey0, d0);
                while key(cx, cy) != s {
                    pts.push((cx, cy));
                    let k = key(cx, cy);
                    let Some(cands) = em.get_mut(&k) else { break };
                    // sharpest left turn first: left, straight, right (never back)
                    let pref = [(cd + 3) % 4, cd, (cd + 1) % 4];
                    let mut chosen = None;
                    for p in pref {
                        if let Some(pos) = cands.iter().position(|&(d, _, _)| d == p) {
                            chosen = Some(cands.swap_remove(pos));
                            break;
                        }
                    }
                    if cands.is_empty() {
                        em.remove(&k);
                    }
                    let Some((d, ex, ey)) = chosen else { break };
                    cx = ex;
                    cy = ey;
                    cd = d;
                }
                // merge collinear runs (axis-aligned by construction)
                if pts.len() >= 4 {
                    let mut m: Loop = Vec::with_capacity(pts.len());
                    let n = pts.len();
                    for i in 0..n {
                        let p = pts[(i + n - 1) % n];
                        let c = pts[i];
                        let nx = pts[(i + 1) % n];
                        let straight = (p.0 == c.0 && c.0 == nx.0) || (p.1 == c.1 && c.1 == nx.1);
                        if !straight {
                            m.push(c);
                        }
                    }
                    if m.len() >= 4 {
                        loops.push(m);
                    }
                }
            }
        }
        out.push(loops);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 2x2 block inside a field: one square loop of 4 vertices for the block, and the
    // field gets an outer rectangle plus the block as a hole.
    #[test]
    fn traces_square_and_hole() {
        let (w, h) = (6usize, 5usize);
        let mut labels = vec![0u32; w * h];
        for y in 2..4 {
            for x in 2..4 {
                labels[y * w + x] = 1;
            }
        }
        let c = region_contours(&labels, w, h, 2);
        assert_eq!(c[1].len(), 1, "block is one loop");
        assert_eq!(c[1][0].len(), 4, "block loop is a square");
        assert_eq!(c[0].len(), 2, "field has outer boundary + hole");
        let mut sizes: Vec<usize> = c[0].iter().map(|l| l.len()).collect();
        sizes.sort();
        assert_eq!(sizes, vec![4, 4]);
    }

    // Disconnected islands of one label become separate loops.
    #[test]
    fn traces_disconnected_islands() {
        let (w, h) = (8usize, 3usize);
        let mut labels = vec![0u32; w * h];
        labels[1 * w + 1] = 1;
        labels[1 * w + 5] = 1;
        let c = region_contours(&labels, w, h, 2);
        assert_eq!(c[1].len(), 2, "two islands, two loops");
    }
}
