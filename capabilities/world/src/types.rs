use serde::{Deserialize, Serialize};

/// A 2D point in world space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance_to(self, other: Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn distance_sq(self, other: Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }

    pub fn midpoint(self, other: Point) -> Point {
        Point {
            x: (self.x + other.x) * 0.5,
            y: (self.y + other.y) * 0.5,
        }
    }

    pub fn lerp(self, other: Point, t: f64) -> Point {
        Point {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
        }
    }

    /// Snap to a grid resolution.
    pub fn snap(self, resolution: f64) -> Point {
        Point {
            x: (self.x / resolution).round() * resolution,
            y: (self.y / resolution).round() * resolution,
        }
    }
}

impl std::ops::Add for Point {
    type Output = Point;
    fn add(self, rhs: Point) -> Point {
        Point::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Point {
    type Output = Point;
    fn sub(self, rhs: Point) -> Point {
        Point::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl std::ops::Mul<f64> for Point {
    type Output = Point;
    fn mul(self, s: f64) -> Point {
        Point::new(self.x * s, self.y * s)
    }
}

/// A polygon defined by its vertices in order (CCW or CW).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Polygon {
    pub vertices: Vec<Point>,
}

impl Polygon {
    pub fn new(vertices: Vec<Point>) -> Self {
        Self { vertices }
    }

    /// Signed area (positive if CCW, negative if CW).
    pub fn signed_area(&self) -> f64 {
        let n = self.vertices.len();
        if n < 3 {
            return 0.0;
        }
        let mut area = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            area += self.vertices[i].x * self.vertices[j].y;
            area -= self.vertices[j].x * self.vertices[i].y;
        }
        area * 0.5
    }

    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }

    pub fn centroid(&self) -> Point {
        let n = self.vertices.len();
        if n == 0 {
            return Point::new(0.0, 0.0);
        }
        let mut cx = 0.0;
        let mut cy = 0.0;
        let mut a = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            let cross = self.vertices[i].x * self.vertices[j].y
                - self.vertices[j].x * self.vertices[i].y;
            cx += (self.vertices[i].x + self.vertices[j].x) * cross;
            cy += (self.vertices[i].y + self.vertices[j].y) * cross;
            a += cross;
        }
        a *= 0.5;
        if a.abs() < 1e-12 {
            // Degenerate — fall back to average
            let sx: f64 = self.vertices.iter().map(|v| v.x).sum();
            let sy: f64 = self.vertices.iter().map(|v| v.y).sum();
            return Point::new(sx / n as f64, sy / n as f64);
        }
        Point::new(cx / (6.0 * a), cy / (6.0 * a))
    }

    /// Test whether a point lies inside the polygon (ray casting).
    pub fn contains(&self, p: Point) -> bool {
        let n = self.vertices.len();
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let vi = self.vertices[i];
            let vj = self.vertices[j];
            if ((vi.y > p.y) != (vj.y > p.y))
                && (p.x < (vj.x - vi.x) * (p.y - vi.y) / (vj.y - vi.y) + vi.x)
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// Bounding box: (min_x, min_y, max_x, max_y).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for v in &self.vertices {
            min_x = min_x.min(v.x);
            min_y = min_y.min(v.y);
            max_x = max_x.max(v.x);
            max_y = max_y.max(v.y);
        }
        (min_x, min_y, max_x, max_y)
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.vertices.len()
    }

    /// Get edge i as (start, end).
    pub fn edge(&self, i: usize) -> (Point, Point) {
        let n = self.vertices.len();
        (self.vertices[i], self.vertices[(i + 1) % n])
    }
}

/// Line segment between two points.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Segment {
    pub a: Point,
    pub b: Point,
}

impl Segment {
    pub fn new(a: Point, b: Point) -> Self {
        Self { a, b }
    }

    pub fn length(&self) -> f64 {
        self.a.distance_to(self.b)
    }

    pub fn midpoint(&self) -> Point {
        self.a.midpoint(self.b)
    }

    /// Point at parameter t in [0, 1].
    pub fn at(&self, t: f64) -> Point {
        self.a.lerp(self.b, t)
    }

    /// Perpendicular direction (not normalised). Rotates (b-a) by 90°.
    pub fn perpendicular(&self) -> Point {
        let dx = self.b.x - self.a.x;
        let dy = self.b.y - self.a.y;
        Point::new(-dy, dx)
    }

    /// Test intersection with another segment. Returns (t, u) parameters if
    /// the segments intersect, where intersection point = a + t*(b-a).
    pub fn intersect(&self, other: &Segment) -> Option<(f64, f64)> {
        let d1 = Point::new(self.b.x - self.a.x, self.b.y - self.a.y);
        let d2 = Point::new(other.b.x - other.a.x, other.b.y - other.a.y);
        let cross = d1.x * d2.y - d1.y * d2.x;
        if cross.abs() < 1e-12 {
            return None; // Parallel
        }
        let d = Point::new(other.a.x - self.a.x, other.a.y - self.a.y);
        let t = (d.x * d2.y - d.y * d2.x) / cross;
        let u = (d.x * d1.y - d.y * d1.x) / cross;
        if t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0 {
            Some((t, u))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_area() {
        let tri = Polygon::new(vec![
            Point::new(0.0, 0.0),
            Point::new(4.0, 0.0),
            Point::new(0.0, 3.0),
        ]);
        assert!((tri.area() - 6.0).abs() < 1e-10);
    }

    #[test]
    fn point_in_polygon() {
        let sq = Polygon::new(vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ]);
        assert!(sq.contains(Point::new(5.0, 5.0)));
        assert!(!sq.contains(Point::new(15.0, 5.0)));
    }

    #[test]
    fn segment_intersection() {
        let s1 = Segment::new(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        let s2 = Segment::new(Point::new(0.0, 10.0), Point::new(10.0, 0.0));
        let (t, u) = s1.intersect(&s2).unwrap();
        assert!((t - 0.5).abs() < 1e-10);
        assert!((u - 0.5).abs() < 1e-10);
    }

    #[test]
    fn segment_no_intersection() {
        let s1 = Segment::new(Point::new(0.0, 0.0), Point::new(1.0, 0.0));
        let s2 = Segment::new(Point::new(0.0, 1.0), Point::new(1.0, 1.0));
        assert!(s1.intersect(&s2).is_none());
    }
}
