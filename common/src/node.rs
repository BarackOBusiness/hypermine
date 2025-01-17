/*the name of this module is pretty arbitrary at the moment*/

use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

use crate::collision_math::Ray;
use crate::dodeca::Vertex;
use crate::graph::{Graph, NodeId};
use crate::lru_slab::SlotId;
use crate::proto::{BlockUpdate, Position, SerializableVoxelData};
use crate::world::Material;
use crate::worldgen::NodeState;
use crate::{math, Chunks};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId {
    pub node: NodeId,
    pub vertex: Vertex,
}

impl ChunkId {
    pub fn new(node: NodeId, vertex: Vertex) -> Self {
        ChunkId { node, vertex }
    }
}

impl Graph {
    pub fn get_chunk_mut(&mut self, chunk: ChunkId) -> Option<&mut Chunk> {
        Some(&mut self.get_mut(chunk.node).as_mut()?.chunks[chunk.vertex])
    }

    pub fn get_chunk(&self, chunk: ChunkId) -> Option<&Chunk> {
        Some(&self.get(chunk.node).as_ref()?.chunks[chunk.vertex])
    }

    /// Returns the up-direction relative to the given position, or `None` if the
    /// position is in an unpopulated node.
    pub fn get_relative_up(&self, position: &Position) -> Option<na::UnitVector3<f32>> {
        let node = self.get(position.node).as_ref()?;
        Some(na::UnitVector3::new_normalize(
            (math::mtranspose(&position.local) * node.state.up_direction()).xyz(),
        ))
    }

    pub fn get_chunk_neighbor(
        &self,
        chunk: ChunkId,
        coord_axis: CoordAxis,
        coord_direction: CoordDirection,
    ) -> Option<ChunkId> {
        match coord_direction {
            CoordDirection::Plus => Some(ChunkId::new(
                chunk.node,
                chunk.vertex.adjacent_vertices()[coord_axis as usize],
            )),
            CoordDirection::Minus => Some(ChunkId::new(
                self.neighbor(
                    chunk.node,
                    chunk.vertex.canonical_sides()[coord_axis as usize],
                )?,
                chunk.vertex,
            )),
        }
    }

    pub fn get_block_neighbor(
        &self,
        mut chunk: ChunkId,
        mut coords: Coords,
        coord_axis: CoordAxis,
        coord_direction: CoordDirection,
    ) -> Option<(ChunkId, Coords)> {
        if coords[coord_axis] == self.layout().dimension - 1
            && coord_direction == CoordDirection::Plus
        {
            let new_vertex = chunk.vertex.adjacent_vertices()[coord_axis as usize];
            // Permute coordinates based on differences in the canonical orders between the old
            // and new vertex
            let [coord_plane0, coord_plane1] = coord_axis.other_axes();
            let mut new_coords = Coords([0; 3]);
            for current_axis in CoordAxis::iter() {
                if new_vertex.canonical_sides()[current_axis as usize]
                    == chunk.vertex.canonical_sides()[coord_plane0 as usize]
                {
                    new_coords[current_axis] = coords[coord_plane0];
                } else if new_vertex.canonical_sides()[current_axis as usize]
                    == chunk.vertex.canonical_sides()[coord_plane1 as usize]
                {
                    new_coords[current_axis] = coords[coord_plane1];
                } else {
                    new_coords[current_axis] = coords[coord_axis];
                }
            }
            coords = new_coords;
            chunk.vertex = new_vertex;
        } else if coords[coord_axis] == 0 && coord_direction == CoordDirection::Minus {
            chunk.node = self.neighbor(
                chunk.node,
                chunk.vertex.canonical_sides()[coord_axis as usize],
            )?;
        } else {
            coords[coord_axis] = coords[coord_axis].wrapping_add_signed(coord_direction as i8);
        }

        Some((chunk, coords))
    }

    /// Populates a chunk with the given voxel data and ensures that margins are correctly cleared if necessary.
    pub fn populate_chunk(&mut self, chunk: ChunkId, mut new_data: VoxelData, modified: bool) {
        // New solid chunks should have their margin cleared if they are adjacent to any modified chunks.
        // See the function description of VoxelData::clear_margin for why this is necessary.
        if new_data.is_solid() {
            // Loop through all six potential chunk neighbors. If any are modified, the `new_data` should have
            // its margin cleared.
            'outer: for coord_axis in CoordAxis::iter() {
                for coord_direction in CoordDirection::iter() {
                    if let Some(chunk_id) =
                        self.get_chunk_neighbor(chunk, coord_axis, coord_direction)
                    {
                        if let Chunk::Populated { modified: true, .. } = self[chunk_id] {
                            new_data.clear_margin(self.layout().dimension);
                            break 'outer;
                        }
                    }
                }
            }
        }

        // Existing adjacent solid chunks should have their margins cleared if the chunk we're populating is modified.
        // See the function description of VoxelData::clear_margin for why this is necessary.
        if modified {
            self.clear_adjacent_solid_chunk_margins(chunk);
        }

        // After clearing any margins we needed to clear, we can now insert the data into the graph
        *self.get_chunk_mut(chunk).unwrap() = Chunk::Populated {
            voxels: new_data,
            modified,
            surface: None,
            old_surface: None,
        };
    }

    /// Tries to update the block at the given position to the given material.
    /// Fails and returns false if the chunk is not populated yet.
    #[must_use]
    pub fn update_block(&mut self, block_update: &BlockUpdate) -> bool {
        let dimension = self.layout().dimension;

        // Update the block
        let Some(Chunk::Populated {
            voxels,
            modified,
            surface,
            old_surface,
        }) = self.get_chunk_mut(block_update.chunk_id)
        else {
            return false;
        };
        if voxels.is_solid() {
            voxels.clear_margin(dimension);
        }
        let voxel = voxels
            .data_mut(dimension)
            .get_mut(block_update.coords.to_index(dimension))
            .expect("coords are in-bounds");

        *voxel = block_update.new_material;
        *modified = true;
        *old_surface = surface.take().or(*old_surface);

        self.clear_adjacent_solid_chunk_margins(block_update.chunk_id);
        true
    }

    /// Clears margins from any populated and solid adjacent chunks. When a chunk is modified, this function should
    /// be called on that chunk to ensure that adjacent chunks are rendered, since they can no longer be assumed to be
    /// hidden by world generation.
    fn clear_adjacent_solid_chunk_margins(&mut self, chunk: ChunkId) {
        for coord_axis in CoordAxis::iter() {
            for coord_direction in CoordDirection::iter() {
                if let Some(chunk_id) = self.get_chunk_neighbor(chunk, coord_axis, coord_direction)
                {
                    // We only need to clear margins from populated chunks.
                    let _ = self.clear_solid_chunk_margin(chunk_id);
                }
            }
        }
    }

    /// Tries to clear the margins of the given chunk. Fails and returns false if the
    /// chunk is not populated yet. Succeeds and returns true if the chunk is not Solid, as the
    /// chunk is assumed to have empty margins already.
    #[must_use]
    fn clear_solid_chunk_margin(&mut self, chunk: ChunkId) -> bool {
        let dimension = self.layout().dimension;
        let Some(Chunk::Populated {
            voxels,
            surface,
            old_surface,
            ..
        }) = self.get_chunk_mut(chunk)
        else {
            return false;
        };

        if voxels.is_solid() {
            voxels.clear_margin(dimension);
            *old_surface = surface.take().or(*old_surface);
        }
        true
    }
}

impl Index<ChunkId> for Graph {
    type Output = Chunk;

    fn index(&self, chunk: ChunkId) -> &Chunk {
        self.get_chunk(chunk).unwrap()
    }
}

impl IndexMut<ChunkId> for Graph {
    fn index_mut(&mut self, chunk: ChunkId) -> &mut Chunk {
        self.get_chunk_mut(chunk).unwrap()
    }
}

/// Coordinates for a discrete voxel within a chunk, not including margins
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coords(pub [u8; 3]);

impl Coords {
    /// Returns the array index in `VoxelData` corresponding to these coordinates
    pub fn to_index(&self, chunk_size: u8) -> usize {
        let chunk_size_with_margin = chunk_size as usize + 2;
        (self.0[0] as usize + 1)
            + (self.0[1] as usize + 1) * chunk_size_with_margin
            + (self.0[2] as usize + 1) * chunk_size_with_margin.pow(2)
    }
}

impl Index<CoordAxis> for Coords {
    type Output = u8;

    fn index(&self, coord_axis: CoordAxis) -> &u8 {
        self.0.index(coord_axis as usize)
    }
}

impl IndexMut<CoordAxis> for Coords {
    fn index_mut(&mut self, coord_axis: CoordAxis) -> &mut u8 {
        self.0.index_mut(coord_axis as usize)
    }
}

pub struct Node {
    pub state: NodeState,
    /// We can only populate chunks which lie within a cube of populated nodes, so nodes on the edge
    /// of the graph always have some `None` chunks.
    pub chunks: Chunks<Chunk>,
}

#[derive(Default)]
pub enum Chunk {
    #[default]
    Fresh,
    Generating,
    Populated {
        voxels: VoxelData,
        modified: bool,
        surface: Option<SlotId>,
        old_surface: Option<SlotId>,
    },
}

pub enum VoxelData {
    Solid(Material),
    Dense(Box<[Material]>),
}

impl VoxelData {
    pub fn data_mut(&mut self, dimension: u8) -> &mut [Material] {
        match *self {
            VoxelData::Dense(ref mut d) => d,
            VoxelData::Solid(mat) => {
                *self = VoxelData::Dense(vec![mat; (usize::from(dimension) + 2).pow(3)].into());
                self.data_mut(dimension)
            }
        }
    }

    pub fn get(&self, index: usize) -> Material {
        match *self {
            VoxelData::Dense(ref d) => d[index],
            VoxelData::Solid(mat) => mat,
        }
    }

    /// Replaces all voxels in the margin of this chunk with the "Void" material. This function is a coarse
    /// way to ensure that chunks are fully rendered when they need to be, avoiding a rendering bug caused
    /// by a voxel's surface failing to render because of a margin being solid.
    /// Until margins are fully implemented, any solid chunk produced by world generation should have its
    /// margins cleared if it, or any chunk adjacent to it, is edited, since otherwise, the margins could
    /// be inaccurate.
    pub fn clear_margin(&mut self, dimension: u8) {
        let data = self.data_mut(dimension);
        let lwm = usize::from(dimension) + 2;
        for z in 0..lwm {
            for y in 0..lwm {
                for x in 0..lwm {
                    if x == 0 || x == lwm - 1 || y == 0 || y == lwm - 1 || z == 0 || z == lwm - 1 {
                        // The current coordinates correspond to a margin point. Set it to void.
                        data[x + y * lwm + z * lwm.pow(2)] = Material::Void;
                    }
                }
            }
        }
    }

    pub fn is_solid(&self) -> bool {
        match *self {
            VoxelData::Dense(_) => false,
            VoxelData::Solid(_) => true,
        }
    }

    /// Returns a `VoxelData` with void margins based on the given `SerializableVoxelData`, or `None` if
    /// the `SerializableVoxelData` came from a `VoxelData` with the wrong dimension.
    pub fn from_serializable(serializable: &SerializableVoxelData, dimension: u8) -> Option<Self> {
        if serializable.voxels.len() != usize::from(dimension).pow(3) {
            return None;
        }

        let mut data = vec![Material::Void; (usize::from(dimension) + 2).pow(3)];
        let mut input_index = 0;
        for z in 0..dimension {
            for y in 0..dimension {
                for x in 0..dimension {
                    // We cannot use a linear copy here because `data` has margins, while `serializable.voxels` does not.
                    data[Coords([x, y, z]).to_index(dimension)] = serializable.voxels[input_index];
                    input_index += 1;
                }
            }
        }
        Some(VoxelData::Dense(data.into_boxed_slice()))
    }

    /// Returns a `SerializableVoxelData` corresponding to `self`. Assumes that`self` is `Dense` and
    /// has the right dimension, as it will panic or return incorrect data otherwise.
    pub fn to_serializable(&self, dimension: u8) -> SerializableVoxelData {
        let VoxelData::Dense(data) = self else {
            panic!("Only dense chunks can be serialized.");
        };

        let mut serializable: Vec<Material> = Vec::with_capacity(usize::from(dimension).pow(3));
        for z in 0..dimension {
            for y in 0..dimension {
                for x in 0..dimension {
                    // We cannot use a linear copy here because `data` has margins, while `serializable.voxels` does not.
                    serializable.push(data[Coords([x, y, z]).to_index(dimension)]);
                }
            }
        }
        SerializableVoxelData {
            voxels: serializable,
        }
    }
}

/// Contains the context needed to know the locations of individual cubes within a chunk in the chunk's coordinate
/// system. A given `ChunkLayout` is uniquely determined by its dimension.
pub struct ChunkLayout {
    dimension: u8,
    dual_to_grid_factor: f32,
}

impl ChunkLayout {
    pub fn new(dimension: u8) -> Self {
        ChunkLayout {
            dimension,
            dual_to_grid_factor: Vertex::dual_to_chunk_factor() as f32 * dimension as f32,
        }
    }

    /// Number of cubes on one axis of the chunk.
    #[inline]
    pub fn dimension(&self) -> u8 {
        self.dimension
    }

    /// Scale by this to convert dual coordinates to homogeneous grid coordinates.
    #[inline]
    pub fn dual_to_grid_factor(&self) -> f32 {
        self.dual_to_grid_factor
    }

    /// Converts a single coordinate from dual coordinates in the Klein-Beltrami model to an integer coordinate
    /// suitable for voxel lookup. Returns `None` if the coordinate is outside the chunk.
    #[inline]
    pub fn dual_to_voxel(&self, dual_coord: f32) -> Option<u8> {
        let floor_grid_coord = (dual_coord * self.dual_to_grid_factor).floor();

        if !(floor_grid_coord >= 0.0 && floor_grid_coord < self.dimension as f32) {
            None
        } else {
            Some(floor_grid_coord as u8)
        }
    }

    /// Converts a single coordinate from grid coordinates to dual coordiantes in the Klein-Beltrami model. This
    /// can be used to find the positions of voxel gridlines.
    #[inline]
    pub fn grid_to_dual(&self, grid_coord: u8) -> f32 {
        grid_coord as f32 / self.dual_to_grid_factor
    }

    /// Takes in a single grid coordinate and returns a range containing all voxel coordinates surrounding it.
    #[inline]
    pub fn neighboring_voxels(&self, grid_coord: u8) -> impl Iterator<Item = u8> {
        grid_coord.saturating_sub(1)..grid_coord.saturating_add(1).min(self.dimension())
    }
}

/// Ensures that every new node of the given Graph is populated with a [Node] and is
/// ready for world generation.
pub fn populate_fresh_nodes(graph: &mut Graph) {
    let fresh = graph.fresh().to_vec();
    graph.clear_fresh();
    for &node in &fresh {
        populate_node(graph, node);
    }
}

fn populate_node(graph: &mut Graph, node: NodeId) {
    *graph.get_mut(node) = Some(Node {
        state: graph
            .parent(node)
            .and_then(|i| {
                let parent_state = &graph.get(graph.neighbor(node, i)?).as_ref()?.state;
                Some(parent_state.child(graph, node, i))
            })
            .unwrap_or_else(NodeState::root),
        chunks: Chunks::default(),
    });
}

/// Represents a particular axis in a voxel grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordAxis {
    X = 0,
    Y = 1,
    Z = 2,
}

/// Trying to convert a `usize` to a `CoordAxis` returns this struct if the provided
/// `usize` is out-of-bounds
#[derive(Debug, Clone, Copy)]
pub struct CoordAxisOutOfBounds;

impl CoordAxis {
    /// Iterates through the the axes in ascending order
    pub fn iter() -> impl ExactSizeIterator<Item = Self> {
        [Self::X, Self::Y, Self::Z].iter().copied()
    }

    /// Returns the pair axes orthogonal to the current axis
    pub fn other_axes(self) -> [Self; 2] {
        match self {
            Self::X => [Self::Y, Self::Z],
            Self::Y => [Self::Z, Self::X],
            Self::Z => [Self::X, Self::Y],
        }
    }
}

impl TryFrom<usize> for CoordAxis {
    type Error = CoordAxisOutOfBounds;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::X),
            1 => Ok(Self::Y),
            2 => Ok(Self::Z),
            _ => Err(CoordAxisOutOfBounds),
        }
    }
}

/// Represents a direction in a particular axis. This struct is meant to be used with a coordinate axis,
/// so when paired with the X-axis, it represents the postitive X-direction when set to Plus and the
/// negative X-direction when set to Minus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordDirection {
    Plus = 1,
    Minus = -1,
}

impl CoordDirection {
    /// Iterates through the two possible coordinate directions
    pub fn iter() -> impl ExactSizeIterator<Item = Self> {
        [CoordDirection::Plus, CoordDirection::Minus]
            .iter()
            .copied()
    }
}

/// Represents a discretized region in the voxel grid contained by an axis-aligned bounding box.
pub struct VoxelAABB {
    // The bounds are of the form [[x_min, x_max], [y_min, y_max], [z_min, z_max]], using voxel coordinates with a one-block
    // wide margins added on both sides. This helps make sure that that we can detect if the AABB intersects the chunk's boundaries.
    bounds: [[u8; 2]; 3],
}

impl VoxelAABB {
    /// Returns a bounding box that is guaranteed to cover a given radius around a ray segment. Returns None if the
    /// bounding box lies entirely outside the chunk.
    pub fn from_ray_segment_and_radius(
        layout: &ChunkLayout,
        ray: &Ray,
        tanh_distance: f32,
        radius: f32,
    ) -> Option<VoxelAABB> {
        // Convert the ray to grid coordinates
        let grid_start =
            na::Point3::from_homogeneous(ray.position).unwrap() * layout.dual_to_grid_factor();
        let grid_end = na::Point3::from_homogeneous(ray.ray_point(tanh_distance)).unwrap()
            * layout.dual_to_grid_factor();
        // Convert the radius to grid coordinates using a crude conservative estimate
        let max_grid_radius = radius * layout.dual_to_grid_factor();
        let mut bounds = [[0; 2]; 3];
        for axis in 0..3 {
            let grid_min = grid_start[axis].min(grid_end[axis]) - max_grid_radius;
            let grid_max = grid_start[axis].max(grid_end[axis]) + max_grid_radius;
            let voxel_min = (grid_min + 1.0).floor().max(0.0);
            let voxel_max = (grid_max + 1.0)
                .floor()
                .min(layout.dimension() as f32 + 1.0);

            // When voxel_min is greater than dimension or voxel_max is less than 1, the cube does not intersect
            // the chunk.
            if voxel_min > layout.dimension() as f32 || voxel_max < 1.0 {
                return None;
            }

            // We convert to u8 here instead of earlier because out-of-range voxel coordinates can violate casting assumptions.
            bounds[axis] = [voxel_min.floor() as u8, voxel_max.floor() as u8];
        }

        Some(VoxelAABB { bounds })
    }

    /// Iterator over grid points contained in the region, represented as ordered triples
    pub fn grid_points(
        &self,
        axis0: usize,
        axis1: usize,
        axis2: usize,
    ) -> impl Iterator<Item = (u8, u8, u8)> {
        let bounds = self.bounds;
        (bounds[axis0][0]..bounds[axis0][1]).flat_map(move |i| {
            (bounds[axis1][0]..bounds[axis1][1])
                .flat_map(move |j| (bounds[axis2][0]..bounds[axis2][1]).map(move |k| (i, j, k)))
        })
    }

    /// Iterator over grid lines intersecting the region, represented as ordered pairs determining the line's two fixed coordinates
    pub fn grid_lines(&self, axis0: usize, axis1: usize) -> impl Iterator<Item = (u8, u8)> {
        let bounds = self.bounds;
        (bounds[axis0][0]..bounds[axis0][1])
            .flat_map(move |i| (bounds[axis1][0]..bounds[axis1][1]).map(move |j| (i, j)))
    }

    /// Iterator over grid planes intersecting the region, represented as integers determining the plane's fixed coordinate
    pub fn grid_planes(&self, axis: usize) -> impl Iterator<Item = u8> {
        self.bounds[axis][0]..self.bounds[axis][1]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::math;

    use super::*;

    /// Any voxel AABB should at least cover a capsule-shaped region consisting of all points
    /// `radius` units away from the ray's line segment. This region consists of two spheres
    /// and a cylinder. We only test planes because covered lines and points are a strict subset.
    #[test]
    fn voxel_aabb_coverage() {
        let dimension = 12;
        let layout = ChunkLayout::new(dimension);

        // Pick an arbitrary ray by transforming the positive-x-axis ray.
        let ray = na::Rotation3::from_axis_angle(&na::Vector3::z_axis(), 0.4).to_homogeneous()
            * math::translate_along(&na::Vector3::new(0.2, 0.3, 0.1))
            * &Ray::new(na::Vector4::w(), na::Vector4::x());

        let tanh_distance = 0.2;
        let radius = 0.1;

        // We want to test that the whole capsule-shaped region around the ray segment is covered by
        // the AABB. However, the math to test for this is complicated, so we instead check a bunch of
        // spheres along this ray segment.
        let num_ray_test_points = 20;
        let ray_test_points: Vec<_> = (0..num_ray_test_points)
            .map(|i| {
                math::lorentz_normalize(
                    &ray.ray_point(tanh_distance * (i as f32 / (num_ray_test_points - 1) as f32)),
                )
            })
            .collect();

        let aabb =
            VoxelAABB::from_ray_segment_and_radius(&layout, &ray, tanh_distance, radius).unwrap();

        // For variable names and further comments, we use a tuv coordinate system, which
        // is a permuted xyz coordinate system.

        // Test planes in all 3 axes.
        for t_axis in 0..3 {
            let covered_planes: HashSet<_> = aabb.grid_planes(t_axis).collect();

            // Check that all uv-aligned planes that should be covered are covered
            for t in 0..=dimension {
                if covered_planes.contains(&t) {
                    continue;
                }

                let mut plane_normal = na::Vector4::zeros();
                plane_normal[t_axis] = 1.0;
                plane_normal[3] = layout.grid_to_dual(t);
                let plane_normal = math::lorentz_normalize(&plane_normal);

                for test_point in &ray_test_points {
                    assert!(
                        math::mip(test_point, &plane_normal).abs() > radius.sinh(),
                        "Plane not covered: t_axis={t_axis}, t={t}, test_point={test_point:?}",
                    );
                }
            }
        }

        // Test lines in all 3 axes
        for t_axis in 0..3 {
            let u_axis = (t_axis + 1) % 3;
            let v_axis = (u_axis + 1) % 3;
            let covered_lines: HashSet<_> = aabb.grid_lines(u_axis, v_axis).collect();

            // For a given axis, all lines have the same direction, so set up the appropriate vector
            // in advance.
            let mut line_direction = na::Vector4::zeros();
            line_direction[t_axis] = 1.0;
            let line_direction = line_direction;

            // Check that all t-aligned lines that should be covered are covered
            for u in 0..=dimension {
                for v in 0..=dimension {
                    if covered_lines.contains(&(u, v)) {
                        continue;
                    }

                    let mut line_position = na::Vector4::zeros();
                    line_position[u_axis] = layout.grid_to_dual(u);
                    line_position[v_axis] = layout.grid_to_dual(v);
                    line_position[3] = 1.0;
                    let line_position = math::lorentz_normalize(&line_position);

                    for test_point in &ray_test_points {
                        assert!(
                            (math::mip(test_point, &line_position).powi(2)
                                - math::mip(test_point, &line_direction).powi(2))
                            .sqrt()
                                > radius.cosh(),
                            "Line not covered: t_axis={t_axis}, u={u}, v={v}, test_point={test_point:?}",
                        );
                    }
                }
            }
        }

        // Test points
        let covered_points: HashSet<_> = aabb.grid_points(0, 1, 2).collect();

        // Check that all points that should be covered are covered
        for x in 0..=dimension {
            for y in 0..=dimension {
                for z in 0..=dimension {
                    if covered_points.contains(&(x, y, z)) {
                        continue;
                    }

                    let point_position = math::lorentz_normalize(&na::Vector4::new(
                        layout.grid_to_dual(x),
                        layout.grid_to_dual(y),
                        layout.grid_to_dual(z),
                        1.0,
                    ));

                    for test_point in &ray_test_points {
                        assert!(
                            -math::mip(test_point, &point_position) > radius.cosh(),
                            "Point not covered: x={x}, y={y}, z={z}, test_point={test_point:?}",
                        );
                    }
                }
            }
        }
    }
}
