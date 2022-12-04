#![allow(clippy::new_without_default)]

macro_rules! cstr {
    ($x:literal) => {{
        #[allow(unused_unsafe)]
        unsafe {
            std::ffi::CStr::from_bytes_with_nul_unchecked(concat!($x, "\0").as_bytes())
        }
    }};
}

extern crate nalgebra as na;
mod capsule_chunk_ray_tracer;
mod chunk_ray_tracer;
mod config;
mod graph_ray_tracer;
pub mod graphics;
mod lahar_deprecated;
mod loader;
pub mod metrics;
pub mod net;
mod point_chunk_ray_tracer;
mod prediction;
pub mod sim;
mod single_block_sphere_collision_checker;
mod sphere_chunk_ray_tracer;

pub use config::Config;
pub use sim::Sim;

use loader::{Asset, Loader};
use net::Net;
