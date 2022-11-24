use std::f64::consts::TAU;

use enum_map::{enum_map, Enum, EnumMap};
use lazy_static::lazy_static;

use crate::math::{f32or64, HyperboloidMatrix, HyperboloidVector};

lazy_static! {
    static ref NEIGHBORHOOD: EnumMap<Side, EnumMap<Side, bool>> = enum_map! {
        Side::A => enum_map! { Side::E | Side::B => true, _ => false },
        Side::B => enum_map! { Side::A | Side::C => true, _ => false },
        Side::C => enum_map! { Side::B | Side::D => true, _ => false },
        Side::D => enum_map! { Side::C | Side::E => true, _ => false },
        Side::E => enum_map! { Side::D | Side::F => true, _ => false },
        Side::F => enum_map! { Side::E | Side::A => true, _ => false },
    };
    static ref PENTA: Penta = Penta::compute();
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Enum)]
pub enum Side {
    A = 0,
    B = 1,
    C = 2,
    D = 3,
    E = 4,
    F = 5,
}

impl Side {
    pub fn iter() -> impl ExactSizeIterator<Item = Self> {
        use Side::*;
        [A, B, C, D, E, F].iter().cloned()
    }

    #[inline]
    pub fn is_neighbor(self, side: Side) -> bool {
        NEIGHBORHOOD[self][side]
    }

    #[inline]
    pub fn normal(self) -> &'static na::Vector3<f32or64> {
        &PENTA.normals[self]
    }

    #[inline]
    pub fn reflection(self) -> &'static na::Matrix3<f32or64> {
        &PENTA.reflections[self]
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Enum)]
pub enum Vertex {
    AB = 0,
    BC = 1,
    CD = 2,
    DE = 3,
    EF = 4,
    FA = 5,
}

impl Vertex {
    pub fn iter() -> impl ExactSizeIterator<Item = Self> {
        use Vertex::*;
        [AB, BC, CD, DE, EF, FA].iter().cloned()
    }

    #[inline]
    pub fn sides(self) -> [Side; 2] {
        PENTA.vertex_sides[self]
    }

    pub fn adjacent_vertices(self) -> [Vertex; 2] {
        PENTA.vertex_adjacent_vertices[self]
    }

    #[inline]
    pub fn pos(self) -> &'static na::Vector3<f32or64> {
        &PENTA.vertex_pos[self]
    }

    #[inline]
    pub fn square_to_penta(self) -> &'static na::Matrix3<f32or64> {
        &PENTA.square_to_penta[self]
    }

    #[inline]
    pub fn penta_to_square(self) -> &'static na::Matrix3<f32or64> {
        &PENTA.penta_to_square[self]
    }

    #[inline]
    pub fn voxel_to_square_factor() -> f32or64 {
        PENTA.voxel_to_square_factor
    }

    #[inline]
    pub fn square_to_voxel_factor() -> f32or64 {
        PENTA.square_to_voxel_factor
    }

    #[inline]
    pub fn voxel_to_penta(self) -> &'static na::Matrix3<f32or64> {
        &PENTA.voxel_to_penta[self]
    }

    #[inline]
    pub fn penta_to_voxel(self) -> &'static na::Matrix3<f32or64> {
        &PENTA.penta_to_voxel[self]
    }
}

struct Penta {
    vertex_sides: EnumMap<Vertex, [Side; 2]>,
    vertex_adjacent_vertices: EnumMap<Vertex, [Vertex; 2]>,
    normals: EnumMap<Side, na::Vector3<f32or64>>,
    reflections: EnumMap<Side, na::Matrix3<f32or64>>,
    vertex_pos: EnumMap<Vertex, na::Vector3<f32or64>>,
    square_to_penta: EnumMap<Vertex, na::Matrix3<f32or64>>,
    penta_to_square: EnumMap<Vertex, na::Matrix3<f32or64>>,
    voxel_to_square_factor: f32or64,
    square_to_voxel_factor: f32or64,
    voxel_to_penta: EnumMap<Vertex, na::Matrix3<f32or64>>,
    penta_to_voxel: EnumMap<Vertex, na::Matrix3<f32or64>>,
}

impl Penta {
    fn compute() -> Self {
        // Order 4 pentagonal tiling
        // Note: Despite being constants, they are not really configurable, as the rest of the code
        // depends on them being set to their current values, NUM_SIDES = 5 and ORDER = 4
        const NUM_SIDES: usize = 6;
        const ORDER: usize = 4;

        let side_angle = TAU as f32or64 / NUM_SIDES as f32or64;
        let order_angle = TAU as f32or64 / ORDER as f32or64;

        let cos_side_angle = side_angle.cos();
        let cos_order_angle = order_angle.cos();

        let reflection_r = ((1.0 + cos_order_angle) / (1.0 - cos_side_angle)).sqrt();
        let reflection_z = ((cos_side_angle + cos_order_angle) / (1.0 - cos_side_angle)).sqrt();

        let vertex_sides: EnumMap<Vertex, [Side; 2]> = enum_map! {
            Vertex::AB => [Side::A, Side::B],
            Vertex::BC => [Side::B, Side::C],
            Vertex::CD => [Side::C, Side::D],
            Vertex::DE => [Side::D, Side::E],
            Vertex::EF => [Side::E, Side::F],
            Vertex::FA => [Side::F, Side::A],
        };

        let vertex_adjacent_vertices: EnumMap<Vertex, [Vertex; 2]> = enum_map! {
            Vertex::AB => [Vertex::BC, Vertex::FA],
            Vertex::BC => [Vertex::CD, Vertex::AB],
            Vertex::CD => [Vertex::DE, Vertex::BC],
            Vertex::DE => [Vertex::EF, Vertex::CD],
            Vertex::EF => [Vertex::FA, Vertex::DE],
            Vertex::FA => [Vertex::AB, Vertex::EF],
        };

        let mut normals: EnumMap<Side, na::Vector3<f32or64>> = EnumMap::default();
        let mut vertices: EnumMap<Vertex, na::Vector3<f32or64>> = EnumMap::default();
        let mut square_to_penta: EnumMap<Vertex, na::Matrix3<f32or64>> = EnumMap::default();
        let mut voxel_to_penta: EnumMap<Vertex, na::Matrix3<f32or64>> = EnumMap::default();

        for (side, reflection) in normals.iter_mut() {
            let theta = side_angle * (side as usize) as f32or64;
            *reflection = na::Vector3::new(
                reflection_r * theta.cos(),
                reflection_r * theta.sin(),
                reflection_z,
            );
        }

        for (vertex, vertex_pos) in vertices.iter_mut() {
            *vertex_pos =
                normals[vertex_sides[vertex][1]].normal(&normals[vertex_sides[vertex][0]]);
            *vertex_pos /= (-vertex_pos.sqr()).sqrt();
        }

        for (vertex, mat) in square_to_penta.iter_mut() {
            *mat = na::Matrix3::from_columns(&[
                -normals[vertex_sides[vertex][0]],
                -normals[vertex_sides[vertex][1]],
                vertices[vertex],
            ]);
        }

        let penta_to_square = square_to_penta.map(|_, m| m.iso_inverse());

        // I've modified this part to not care how many squares meet at a vertex.
        let voxel_to_square_factor = (penta_to_square[Vertex::BC] * na::Vector3::z()).x / (penta_to_square[Vertex::BC] * na::Vector3::z()).z;
        let square_to_voxel_factor = 1.0 / voxel_to_square_factor;

        for (vertex, mat) in voxel_to_penta.iter_mut() {
            let reflector0 = &normals[vertex_sides[vertex][0]];
            let reflector1 = &normals[vertex_sides[vertex][1]];
            let origin = na::Vector3::new(0.0, 0.0, 1.0);
            *mat = na::Matrix3::from_columns(&[
                -reflector0 * reflector0.z,
                -reflector1 * reflector1.z,
                origin + reflector0 * reflector0.z + reflector1 * reflector1.z,
            ]);
        }

        Penta {
            vertex_sides,
            vertex_adjacent_vertices,
            normals,
            reflections: normals.map(|_, v| v.reflection()),
            vertex_pos: vertices,
            square_to_penta,
            penta_to_square,
            voxel_to_square_factor,
            square_to_voxel_factor,
            voxel_to_penta: square_to_penta
                .map(|_, m| m * na::Matrix3::new_scaling(voxel_to_square_factor)),
            penta_to_voxel: penta_to_square
                .map(|_, m| na::Matrix3::new_scaling(square_to_voxel_factor) * m),
        }
    }
}
