use std::time::Duration;

use fxhash::FxHashMap;
use hecs::Entity;
use tracing::{debug, error, trace};

use crate::{net, prediction::PredictedMotion, Net};
use common::{
    character_controller::CharacterControllerPass,
    graph::{Graph, NodeId},
    node::{DualGraph, Node},
    proto::{self, Character, CharacterInput, Command, Component, Position},
    sanitize_motion_input,
    worldgen::NodeState,
    Chunks, EntityId, GraphEntities, SimConfig, Step,
};

/// Game state
pub struct Sim {
    net: Net,

    // World state
    pub graph: DualGraph,
    pub graph_entities: GraphEntities,
    entity_ids: FxHashMap<EntityId, Entity>,
    pub world: hecs::World,
    pub params: Option<Parameters>,
    pub local_character: Option<Entity>,
    orientation: na::UnitQuaternion<f32>,
    step: Option<Step>,

    // Input state
    since_input_sent: Duration,
    /// Most recent input
    ///
    /// Units are relative to movement speed.
    instantaneous_velocity: na::Vector3<f32>,
    /// Average input over the current time step. The portion of the timestep which has not yet
    /// elapsed is considered to have zero input.
    ///
    /// Units are relative to movement speed.
    average_velocity: na::Vector3<f32>,
    prediction: PredictedMotion,
}

impl Sim {
    pub fn new(net: Net) -> Self {
        Self {
            net,

            graph: Graph::new(),
            graph_entities: GraphEntities::new(),
            entity_ids: FxHashMap::default(),
            world: hecs::World::new(),
            params: None,
            local_character: None,
            orientation: na::one(),
            step: None,

            since_input_sent: Duration::new(0, 0),
            instantaneous_velocity: na::zero(),
            average_velocity: na::zero(),
            prediction: PredictedMotion::new(
                proto::Position {
                    node: NodeId::ROOT,
                    local: na::one(),
                },
                proto::Character {
                    name: "Temp".to_string(),
                    orientation: na::UnitQuaternion::identity(),
                    velocity: na::zero(),
                },
            ),
        }
    }

    pub fn rotate(&mut self, delta: &na::UnitQuaternion<f32>) {
        self.orientation *= delta;
    }

    pub fn velocity(&mut self, v: na::Vector3<f32>) {
        self.instantaneous_velocity = v;
    }

    pub fn params(&self) -> Option<&Parameters> {
        self.params.as_ref()
    }

    pub fn step(&mut self, dt: Duration) {
        self.orientation.renormalize_fast();

        while let Ok(msg) = self.net.incoming.try_recv() {
            self.handle_net(msg);
        }

        if let Some(step_interval) = self
            .params
            .as_ref()
            .map(|x| Duration::from_secs_f32(1.0 / x.sim_config.rate as f32))
        {
            self.since_input_sent += dt;
            if let Some(overflow) = self.since_input_sent.checked_sub(step_interval) {
                // At least one step interval has passed since we last sent input, so it's time to
                // send again.

                // Update average velocity for the time between the last input sample and the end of
                // the previous step. dt > overflow because we check whether a step has elapsed
                // after each increment.
                self.average_velocity += self.instantaneous_velocity
                    * (dt - overflow).as_secs_f32()
                    / step_interval.as_secs_f32();

                // Send fresh input
                self.send_input();

                // Reset state for the next step
                if overflow > step_interval {
                    // If it's been more than two timesteps since we last sent input, skip ahead
                    // rather than spamming the server.
                    self.average_velocity = na::zero();
                    self.since_input_sent = Duration::new(0, 0);
                } else {
                    self.average_velocity = self.instantaneous_velocity * overflow.as_secs_f32()
                        / step_interval.as_secs_f32();
                    // Send the next input a little sooner if necessary to stay in sync
                    self.since_input_sent = overflow;
                }
            } else {
                // Update average velocity for the time within the current step
                self.average_velocity +=
                    self.instantaneous_velocity * dt.as_secs_f32() / step_interval.as_secs_f32();
            }
        }
    }

    fn handle_net(&mut self, msg: net::Message) {
        use net::Message::*;
        match msg {
            ConnectionLost(e) => {
                error!("connection lost: {}", e);
            }
            Hello(msg) => {
                self.params = Some(Parameters {
                    character_id: msg.character,
                    sim_config: msg.sim_config,
                });
                // Populate the root node
                populate_fresh_nodes(&mut self.graph);
            }
            Spawns(msg) => self.handle_spawns(msg),
            StateDelta(msg) => {
                // Discard out-of-order messages, taking care to account for step counter wrapping.
                if self.step.map_or(false, |x| x.wrapping_sub(msg.step) >= 0) {
                    return;
                }
                self.step = Some(msg.step);
                for (id, new_pos, new_char) in &msg.positions {
                    self.update_position(msg.latest_input, *id, *new_pos, new_char.clone());
                }
            }
        }
    }

    fn update_position(
        &mut self,
        latest_input: u16,
        id: EntityId,
        new_pos: Position,
        new_char: Character,
    ) {
        if let Some(params) = self.params.as_ref() {
            if params.character_id == id {
                self.prediction.reconcile(
                    &self.graph,
                    &params.sim_config,
                    1.0 / params.sim_config.rate as f32,
                    latest_input,
                    new_pos,
                    new_char.clone(),
                );
            }
        }
        match self.entity_ids.get(&id) {
            None => debug!(%id, "position update for unknown entity"),
            Some(&entity) => match (
                self.world.get::<&mut Position>(entity),
                self.world.get::<&mut Character>(entity),
            ) {
                (Ok(mut pos), Ok(mut char)) => {
                    if pos.node != new_pos.node {
                        self.graph_entities.remove(pos.node, entity);
                        self.graph_entities.insert(new_pos.node, entity);
                    }
                    *pos = new_pos;
                    *char = new_char;
                }
                _ => error!(%id, "position update for unpositioned entity {:?}", entity),
            },
        }
    }

    fn handle_spawns(&mut self, msg: proto::Spawns) {
        self.step = self.step.max(Some(msg.step));
        let mut builder = hecs::EntityBuilder::new();
        for (id, components) in msg.spawns {
            self.spawn(&mut builder, id, components);
        }
        for &id in &msg.despawns {
            match self.entity_ids.get(&id) {
                Some(&entity) => self.destroy(entity),
                None => error!(%id, "despawned unknown entity"),
            }
        }
        if !msg.nodes.is_empty() {
            trace!(count = msg.nodes.len(), "adding nodes");
        }
        for node in &msg.nodes {
            self.graph.insert_child(node.parent, node.side);
        }
        populate_fresh_nodes(&mut self.graph);
    }

    fn spawn(
        &mut self,
        builder: &mut hecs::EntityBuilder,
        id: EntityId,
        components: Vec<Component>,
    ) {
        trace!(%id, "spawning entity");
        builder.add(id);
        let mut node = None;
        for component in components {
            use common::proto::Component::*;
            match component {
                Character(x) => {
                    builder.add(x);
                }
                Position(x) => {
                    node = Some(x.node);
                    builder.add(x);
                }
            };
        }
        let entity = self.world.spawn(builder.build());
        if let Some(node) = node {
            self.graph_entities.insert(node, entity);
        }
        if id == self.params.as_ref().unwrap().character_id {
            self.local_character = Some(entity);
        }
        if let Some(x) = self.entity_ids.insert(id, entity) {
            self.destroy_idless(x);
            error!(%id, "id collision");
        }
    }

    fn send_input(&mut self) {
        let velocity = sanitize_motion_input(self.orientation * self.average_velocity);
        let params = self.params.as_ref().unwrap();
        let player_input = CharacterInput {
            movement: velocity,
            orientation: self.orientation,
            attempt_jump: false,
            no_clip: true,
        };
        let generation = self.prediction.push(
            &self.graph,
            &params.sim_config,
            1.0 / params.sim_config.rate as f32,
            &player_input,
        );

        // Any failure here will be better handled in handle_net's ConnectionLost case
        let _ = self.net.outgoing.send(Command {
            generation,
            player_input,
        });
    }

    pub fn view(&self) -> Position {
        let mut result = *self.prediction.predicted_position();
        let mut predicted_character = self.prediction.predicted_character().clone();
        if let Some(ref params) = self.params {
            // Apply input that hasn't been sent yet
            let predicted_input = CharacterInput {
                movement: self.orientation * self.average_velocity
                    / self.since_input_sent.as_secs_f32()
                    / params.sim_config.rate as f32,
                orientation: self.orientation,
                attempt_jump: false,
                no_clip: true,
            };
            println!("{}", predicted_input.movement.norm());
            CharacterControllerPass {
                position: &mut result,
                character: &mut predicted_character,
                input: &predicted_input,
                graph: &self.graph,
                config: &params.sim_config,
                dt_seconds: self.since_input_sent.as_secs_f32(),
            }
            .step();
        }
        result.local *= self.orientation.to_homogeneous();
        result
    }

    /// Destroy all aspects of an entity
    fn destroy(&mut self, entity: Entity) {
        let id = *self
            .world
            .get::<&EntityId>(entity)
            .expect("destroyed nonexistent entity");
        self.entity_ids.remove(&id);
        self.destroy_idless(entity);
    }

    /// Destroy an entity without an EntityId mapped
    fn destroy_idless(&mut self, entity: Entity) {
        if let Ok(position) = self.world.get::<&Position>(entity) {
            self.graph_entities.remove(position.node, entity);
        }
        self.world
            .despawn(entity)
            .expect("destroyed nonexistent entity");
    }
}

/// Simulation details received on connect
pub struct Parameters {
    pub sim_config: SimConfig,
    pub character_id: EntityId,
}

fn populate_fresh_nodes(graph: &mut DualGraph) {
    let fresh = graph.fresh().to_vec();
    graph.clear_fresh();
    for &node in &fresh {
        populate_node(graph, node);
    }
}

fn populate_node(graph: &mut DualGraph, node: NodeId) {
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
