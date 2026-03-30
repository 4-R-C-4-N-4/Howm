//! being.form → Astral Geometry translation.
//!
//! Maps HDL silhouette/composition/scale/detail traits to Astral's SDF
//! primitives (sphere, box, cylinder) with appropriate transforms.

use serde::{Deserialize, Serialize};

use crate::hdl::traits::{DescriptionGraph, Trait};

/// Astral Vec3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    pub fn uniform(v: f64) -> Self {
        Self { x: v, y: v, z: v }
    }
}

/// Astral Geometry — mirrors the TypeScript Geometry union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    #[serde(rename = "sphere")]
    Sphere { radius: f64 },
    #[serde(rename = "box")]
    Box { size: Vec3 },
    #[serde(rename = "plane")]
    Plane { normal: Vec3 },
    #[serde(rename = "cylinder")]
    Cylinder { radius: f64, height: f64 },
}

/// Astral Transform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Transform {
    pub fn at(x: f64, y: f64, z: f64) -> Self {
        Self {
            position: Vec3::new(x, y, z),
            rotation: Vec3::new(0.0, 0.0, 0.0),
            scale: Vec3::uniform(1.0),
        }
    }

    pub fn with_scale(mut self, sx: f64, sy: f64, sz: f64) -> Self {
        self.scale = Vec3::new(sx, sy, sz);
        self
    }

    pub fn with_rotation_y(mut self, rad: f64) -> Self {
        self.rotation.y = rad;
        self
    }
}

/// Resolve `being.form` traits into an Astral Geometry + Transform.
///
/// Buildings with explicit footprints use box geometry directly.
/// Parametric entities (creatures, fixtures, flora) derive geometry from
/// silhouette × scale × composition.
pub fn resolve_geometry(graph: &DescriptionGraph) -> (Geometry, Vec3) {
    let silhouette = find_trait(graph, "being.form.silhouette");
    let scale_trait = find_trait(graph, "being.form.scale");

    let factor = scale_trait
        .and_then(|t| t.params.get("factor"))
        .copied()
        .unwrap_or(1.0);

    let silhouette_term = silhouette.map(|t| t.term.as_str()).unwrap_or("compact");

    match silhouette_term {
        "tall" => {
            let h = 1.0 + factor * 3.0;
            let r = 0.3 + factor * 0.3;
            (Geometry::Cylinder { radius: r, height: h }, Vec3::new(1.0, 1.0, 1.0))
        }
        "wide" => {
            let w = 1.0 + factor * 2.0;
            let h = 0.5 + factor * 0.5;
            (Geometry::Box { size: Vec3::new(w, h, w * 0.8) }, Vec3::new(1.0, 1.0, 1.0))
        }
        "compact" => {
            let r = 0.3 + factor * 0.5;
            (Geometry::Sphere { radius: r }, Vec3::new(1.0, 1.0, 1.0))
        }
        "trailing" => {
            let h = 0.8 + factor * 2.0;
            let r = 0.15 + factor * 0.15;
            (Geometry::Cylinder { radius: r, height: h }, Vec3::new(1.0, 1.0, 1.0))
        }
        "irregular" => {
            // Union of offset spheres — represented as a sphere with scale distortion
            let r = 0.4 + factor * 0.4;
            (Geometry::Sphere { radius: r }, Vec3::new(1.2, 0.8, 1.0))
        }
        "columnar" => {
            let h = 1.5 + factor * 3.0;
            let r = 0.2 + factor * 0.2;
            (Geometry::Cylinder { radius: r, height: h }, Vec3::new(1.0, 1.0, 1.0))
        }
        _ => {
            let r = 0.3 + factor * 0.5;
            (Geometry::Sphere { radius: r }, Vec3::new(1.0, 1.0, 1.0))
        }
    }
}

/// Resolve building geometry from an explicit footprint polygon + height.
/// Buildings bypass parametric resolution — they use extruded box from
/// the centroid and bounding dimensions of the footprint.
pub fn resolve_building_geometry(
    footprint: &[crate::types::Point],
    height: f64,
) -> (Geometry, Transform) {
    if footprint.is_empty() {
        return (
            Geometry::Box { size: Vec3::new(2.0, 3.0, 2.0) },
            Transform::at(0.0, 1.5, 0.0),
        );
    }

    let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
    let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);
    for p in footprint {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    let width = (max_x - min_x).max(0.5);
    let depth = (max_y - min_y).max(0.5);
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;

    (
        Geometry::Box {
            size: Vec3::new(width, height, depth),
        },
        Transform::at(cx, height * 0.5, cy),
    )
}

fn find_trait<'a>(graph: &'a DescriptionGraph, path: &str) -> Option<&'a Trait> {
    graph.traits.iter().find(|t| t.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdl::traits::{DescriptionGraph, Trait};

    #[test]
    fn tall_silhouette_produces_cylinder() {
        let mut g = DescriptionGraph::new();
        g.push_trait(Trait::new("being.form.silhouette", "tall").with_param("aspect", 0.5));
        g.push_trait(Trait::new("being.form.scale", "large").with_param("factor", 1.5));

        let (geo, _scale) = resolve_geometry(&g);
        match geo {
            Geometry::Cylinder { radius, height } => {
                assert!(height > 3.0, "tall should be tall: {}", height);
                assert!(radius < 1.0, "tall should be thin: {}", radius);
            }
            _ => panic!("expected cylinder for tall silhouette"),
        }
    }

    #[test]
    fn compact_silhouette_produces_sphere() {
        let mut g = DescriptionGraph::new();
        g.push_trait(Trait::new("being.form.silhouette", "compact"));
        g.push_trait(Trait::new("being.form.scale", "moderate").with_param("factor", 1.0));

        let (geo, _) = resolve_geometry(&g);
        assert!(matches!(geo, Geometry::Sphere { .. }));
    }

    #[test]
    fn building_geometry_from_footprint() {
        let footprint = vec![
            crate::types::Point::new(0.0, 0.0),
            crate::types::Point::new(10.0, 0.0),
            crate::types::Point::new(10.0, 8.0),
            crate::types::Point::new(0.0, 8.0),
        ];
        let (geo, transform) = resolve_building_geometry(&footprint, 6.0);
        match geo {
            Geometry::Box { size } => {
                assert!((size.x - 10.0).abs() < 0.01);
                assert!((size.y - 6.0).abs() < 0.01);
                assert!((size.z - 8.0).abs() < 0.01);
            }
            _ => panic!("expected box for building"),
        }
        assert!((transform.position.x - 5.0).abs() < 0.01);
        assert!((transform.position.y - 3.0).abs() < 0.01);
    }
}
