//! Scene compiler — translates HDL description graphs into Astral Scene JSON.
//!
//! Phase R1: bridge between the world capability and the Astral renderer.
//! The output matches Astral's Scene/Entity/Material/Geometry types exactly.

pub mod geometry;
pub mod material;
pub mod compiler;
pub mod map;
