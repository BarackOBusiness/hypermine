use serde::{Deserialize, Serialize};

use crate::{
    dodeca::{self, Vertex},
    graph::NodeId,
    node::Coords,
    world::Material,
    EntityId, SimConfig, Step,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientHello {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerHello {
    pub character: EntityId,
    pub sim_config: SimConfig,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone)]
pub struct Position {
    pub node: NodeId,
    pub local: na::Matrix4<f32>,
}

impl Position {
    pub fn origin() -> Self {
        Self {
            node: NodeId::ROOT,
            local: na::Matrix4::identity(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDelta {
    pub step: Step,
    /// Highest input generation received prior to `step`
    pub latest_input: u16,
    pub positions: Vec<(EntityId, Position)>,
    pub character_states: Vec<(EntityId, CharacterState)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterState {
    pub velocity: na::Vector3<f32>,
    pub orientation: na::UnitQuaternion<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Spawns {
    pub step: Step,
    pub spawns: Vec<(EntityId, Vec<Component>)>,
    pub despawns: Vec<EntityId>,
    pub nodes: Vec<FreshNode>,
    pub block_updates: Vec<BlockUpdate>,
    pub modified_chunks: Vec<(GlobalChunkId, SerializableVoxelData)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    pub generation: u16,
    pub character_input: CharacterInput,
    pub orientation: na::UnitQuaternion<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInput {
    /// Relative to the character's current position, excluding orientation
    pub movement: na::Vector3<f32>,
    pub no_clip: bool,
    pub block_update: Option<BlockUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GlobalChunkId {
    pub node_hash: u128,
    pub vertex: Vertex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockUpdate {
    pub chunk_id: GlobalChunkId,
    pub coords: Coords,
    pub new_material: Material,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SerializableVoxelData {
    pub voxels: Vec<Material>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Component {
    Character(Character),
    Position(Position),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FreshNode {
    /// The side joining the new node to `parent`
    pub side: dodeca::Side,
    pub parent: NodeId,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Character {
    pub name: String,
    pub state: CharacterState,
}
