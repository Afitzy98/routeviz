use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

// Render-layer coordinates, separate from `Token`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub fn circle_layout(n: usize, radius: f64) -> Vec<Point> {
    if n == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let theta = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            Point {
                x: radius * theta.cos(),
                y: radius * theta.sin(),
            }
        })
        .collect()
}

// Fruchterman-Reingold force-directed layout. All-pairs repulsion +
// edge attraction with simulated-annealing cooling. Output is
// clamped into [-bounds/2, bounds/2]² and seeded for determinism.
// O(V² · iterations).
pub fn fruchterman_reingold_layout(
    num_nodes: usize,
    adjacency: &[Vec<usize>],
    bounds: f64,
    iterations: usize,
    seed: u64,
) -> Vec<Point> {
    if num_nodes == 0 {
        return Vec::new();
    }
    assert_eq!(
        adjacency.len(),
        num_nodes,
        "adjacency must have one entry per node"
    );

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let half = bounds / 2.0;
    // FR ideal-spring length.
    let area = bounds * bounds;
    let k = (area / num_nodes as f64).sqrt();

    let mut positions: Vec<Point> = (0..num_nodes)
        .map(|_| Point {
            x: rng.random_range(-half..half),
            y: rng.random_range(-half..half),
        })
        .collect();

    if num_nodes == 1 {
        positions[0] = Point { x: 0.0, y: 0.0 };
        return positions;
    }

    // Cools linearly to zero by the final iteration.
    let mut temperature = bounds / 10.0;
    let cooling = if iterations > 0 {
        temperature / iterations as f64
    } else {
        0.0
    };

    let mut disp: Vec<(f64, f64)> = vec![(0.0, 0.0); num_nodes];

    for _ in 0..iterations {
        for d in disp.iter_mut() {
            *d = (0.0, 0.0);
        }

        // Repulsive forces — all pairs.
        for i in 0..num_nodes {
            for j in 0..num_nodes {
                if i == j {
                    continue;
                }
                let dx = positions[i].x - positions[j].x;
                let dy = positions[i].y - positions[j].y;
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = k * k / d;
                disp[i].0 += dx / d * force;
                disp[i].1 += dy / d * force;
            }
        }

        // Attractive forces — each undirected edge once (`u < v`).
        for u in 0..num_nodes {
            for &v in &adjacency[u] {
                if v <= u || v >= num_nodes {
                    continue;
                }
                let dx = positions[u].x - positions[v].x;
                let dy = positions[u].y - positions[v].y;
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = d * d / k;
                disp[u].0 -= dx / d * force;
                disp[u].1 -= dy / d * force;
                disp[v].0 += dx / d * force;
                disp[v].1 += dy / d * force;
            }
        }

        // Apply displacement capped by current temperature; clamp to bounds.
        for i in 0..num_nodes {
            let (dx, dy) = disp[i];
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            let step = d.min(temperature);
            positions[i].x = (positions[i].x + dx / d * step).clamp(-half, half);
            positions[i].y = (positions[i].y + dy / d * step).clamp(-half, half);
        }

        temperature = (temperature - cooling).max(0.0);
    }

    positions
}

// Two concentric rings (hubs inside, spokes outside), both starting
// at 12 o'clock. positions[i] matches is_hub[i].
pub fn hub_spoke_layout(is_hub: &[bool], inner_radius: f64, outer_radius: f64) -> Vec<Point> {
    let n = is_hub.len();
    if n == 0 {
        return Vec::new();
    }
    let hub_count = is_hub.iter().filter(|&&h| h).count();
    let spoke_count = n - hub_count;

    let mut positions = vec![Point { x: 0.0, y: 0.0 }; n];
    let mut hub_rank = 0usize;
    let mut spoke_rank = 0usize;
    let start = -std::f64::consts::FRAC_PI_2;

    for (i, &hub) in is_hub.iter().enumerate() {
        let (rank, count, radius) = if hub {
            let out = (hub_rank, hub_count, inner_radius);
            hub_rank += 1;
            out
        } else {
            let out = (spoke_rank, spoke_count, outer_radius);
            spoke_rank += 1;
            out
        };
        let theta = if count == 0 {
            0.0
        } else {
            start + 2.0 * std::f64::consts::PI * (rank as f64) / (count as f64)
        };
        positions[i] = Point {
            x: radius * theta.cos(),
            y: radius * theta.sin(),
        };
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_layout_produces_n_points() {
        let pts = circle_layout(8, 100.0);
        assert_eq!(pts.len(), 8);
    }

    #[test]
    fn circle_layout_points_are_on_the_circle() {
        let radius = 300.0;
        for pt in circle_layout(20, radius) {
            let r = (pt.x * pt.x + pt.y * pt.y).sqrt();
            assert!((r - radius).abs() < 1e-9, "point {:?} has radius {}", pt, r);
        }
    }

    #[test]
    fn circle_layout_points_are_distinct() {
        let pts = circle_layout(10, 100.0);
        for i in 0..pts.len() {
            for j in (i + 1)..pts.len() {
                assert_ne!(pts[i], pts[j]);
            }
        }
    }

    #[test]
    fn circle_layout_handles_zero() {
        assert!(circle_layout(0, 100.0).is_empty());
    }

    #[test]
    fn hub_spoke_layout_places_hubs_on_inner_ring_spokes_on_outer() {
        let is_hub = [true, true, false, false, false];
        let pts = hub_spoke_layout(&is_hub, 100.0, 400.0);
        assert_eq!(pts.len(), 5);
        for (i, p) in pts.iter().enumerate() {
            let r = (p.x * p.x + p.y * p.y).sqrt();
            let expected = if is_hub[i] { 100.0 } else { 400.0 };
            assert!(
                (r - expected).abs() < 1e-6,
                "index {} radius {} expected {}",
                i,
                r,
                expected
            );
        }
    }

    #[test]
    fn hub_spoke_layout_preserves_input_indexing() {
        // The token at index 2 here is a hub sandwiched between spokes —
        // the function must put it on the inner ring without renumbering.
        let is_hub = [false, false, true, false, false];
        let pts = hub_spoke_layout(&is_hub, 100.0, 400.0);
        let r2 = (pts[2].x * pts[2].x + pts[2].y * pts[2].y).sqrt();
        assert!((r2 - 100.0).abs() < 1e-6);
    }

    #[test]
    fn hub_spoke_layout_handles_all_hubs() {
        let is_hub = [true; 4];
        let pts = hub_spoke_layout(&is_hub, 100.0, 400.0);
        for p in &pts {
            let r = (p.x * p.x + p.y * p.y).sqrt();
            assert!((r - 100.0).abs() < 1e-6);
        }
    }

    #[test]
    fn hub_spoke_layout_handles_all_spokes() {
        let is_hub = [false; 4];
        let pts = hub_spoke_layout(&is_hub, 100.0, 400.0);
        for p in &pts {
            let r = (p.x * p.x + p.y * p.y).sqrt();
            assert!((r - 400.0).abs() < 1e-6);
        }
    }

    #[test]
    fn hub_spoke_layout_handles_empty() {
        assert!(hub_spoke_layout(&[], 100.0, 400.0).is_empty());
    }

    #[test]
    fn fruchterman_reingold_lays_out_within_bounds() {
        let adjacency = vec![vec![1, 2], vec![0, 2], vec![0, 1]];
        let pts = fruchterman_reingold_layout(3, &adjacency, 400.0, 50, 42);
        assert_eq!(pts.len(), 3);
        for p in &pts {
            assert!(p.x >= -200.0 - 1.0 && p.x <= 200.0 + 1.0);
            assert!(p.y >= -200.0 - 1.0 && p.y <= 200.0 + 1.0);
        }
    }

    #[test]
    fn fruchterman_reingold_is_deterministic_by_seed() {
        let adjacency = vec![vec![1], vec![0, 2], vec![1]];
        let a = fruchterman_reingold_layout(3, &adjacency, 400.0, 30, 42);
        let b = fruchterman_reingold_layout(3, &adjacency, 400.0, 30, 42);
        let c = fruchterman_reingold_layout(3, &adjacency, 400.0, 30, 43);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn fruchterman_reingold_handles_empty_and_single() {
        assert!(fruchterman_reingold_layout(0, &[], 400.0, 10, 42).is_empty());
        let one = fruchterman_reingold_layout(1, &[vec![]], 400.0, 10, 42);
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn fruchterman_reingold_pulls_connected_nodes_closer_than_unconnected() {
        // Four nodes: two connected (0-1) and two isolated (2, 3).
        // After FR, the connected pair should sit much closer to each
        // other than any isolated-isolated or isolated-connected pair.
        let adjacency = vec![vec![1], vec![0], vec![], vec![]];
        let pts = fruchterman_reingold_layout(4, &adjacency, 400.0, 100, 42);
        let d = |i: usize, j: usize| {
            let dx = pts[i].x - pts[j].x;
            let dy = pts[i].y - pts[j].y;
            (dx * dx + dy * dy).sqrt()
        };
        let connected = d(0, 1);
        let unconnected = d(2, 3);
        assert!(
            connected < unconnected,
            "expected connected pair (d={connected}) closer than isolated pair (d={unconnected})"
        );
    }
}
