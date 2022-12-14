use crate::{
    graph::Graph,
    math,
    proto::{Character, Position, CharacterInput},
    sanitize_motion_input, SimConfig,
};

pub struct CharacterControllerPass<'a, T> {
    pub position: &'a mut Position,
    pub character: &'a mut Character,
    pub input: &'a CharacterInput,
    pub graph: &'a Graph<T>,
    pub config: &'a SimConfig,
    pub dt_seconds: f32,
}

impl<T> CharacterControllerPass<'_, T> {
    pub fn step(&mut self) {
        let movement = sanitize_motion_input(self.input.movement);

        if self.input.no_clip {
            self.character.velocity = na::Vector3::zeros();
            self.position.local *= math::translate_along(
                &(movement * self.config.no_clip_movement_speed * self.dt_seconds),
            );
        } else {
            let old_velocity = self.character.velocity;

            // Update velocity
            let current_to_target_velocity =
                movement * self.config.max_ground_speed - self.character.velocity;
            let max_delta_velocity = self.config.ground_acceleration * self.dt_seconds;
            if current_to_target_velocity.norm_squared() > max_delta_velocity.powi(2) {
                self.character.velocity +=
                    current_to_target_velocity.normalize() * max_delta_velocity;
            } else {
                self.character.velocity += current_to_target_velocity;
            }

            // Update position
            self.position.local *=
                math::translate_along(&((self.character.velocity + old_velocity) * 0.5 * self.dt_seconds));
        }

        // Renormalize
        self.position.local = math::renormalize_isometry(&self.position.local);
        let (next_node, transition_xf) = self
            .graph
            .normalize_transform(self.position.node, &self.position.local);
        if next_node != self.position.node {
            self.position.node = next_node;
            self.position.local = transition_xf * self.position.local;
        }
    }
}
