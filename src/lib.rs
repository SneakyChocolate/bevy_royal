pub use avian2d::prelude::*;
use bincode::{Decode, Encode};
pub use rand::Rng;
pub use bevy::window::PrimaryWindow;
pub use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    post_process::bloom::{Bloom, BloomCompositeMode},
    prelude::*,
};

pub type NetIDType = u128;

#[derive(Resource)]
pub struct CursorPos(pub Vec2);

#[derive(Component)]
pub struct Velocity(pub Vec2);

#[derive(Component)]
pub struct Radius(pub f32);

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct Alive(pub bool);

#[derive(Component)]
pub struct Enemy;

pub fn random_velocity() -> Vec2 {
    let mut rng = rand::rng();
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let speed = rng.random_range(50.0..200.0);
    Vec2::from_angle(angle) * speed
}

pub fn random_position(range: f32) -> Vec2 {
    let mut rng = rand::rng();
    Vec2::new(
        rng.random_range(-range..range),
        rng.random_range(-range..range),
    )
}

#[derive(Encode, Decode, Debug, Clone, Copy)]
pub struct MyVec3 {
	x: f32,
	y: f32,
	z: f32,
}

impl Into<Vec3> for MyVec3 {
    fn into(self) -> Vec3 {
    	Vec3::new(self.x, self.y, self.z)
    }
}

impl Into<MyVec3> for Vec3 {
    fn into(self) -> MyVec3 {
    	MyVec3 {
	        x: self.x,
	        y: self.y,
	        z: self.z,
	    }
    }
}

#[derive(Encode, Decode, Debug, Clone, Copy)]
pub struct MyVec2 {
	x: f32,
	y: f32,
}

impl Into<Vec2> for MyVec2 {
    fn into(self) -> Vec2 {
    	Vec2::new(self.x, self.y)
    }
}

impl Into<MyVec2> for Vec2 {
    fn into(self) -> MyVec2 {
    	MyVec2 {
	        x: self.x,
	        y: self.y,
	    }
    }
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct EnemyPackage {
	pub net_id: NetIDType,
	pub position: MyVec3,
	pub radius: f32,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct PlayerPackage {
	pub net_id: NetIDType,
	pub position: MyVec3,
}

#[derive(Encode, Decode, Debug, Clone)]
pub enum ServerMessage {
	Ok(NetIDType), // the id of the player so that it knows which id it is
	UpdateEnemies(Vec<EnemyPackage>),
	UpdatePlayers(Vec<PlayerPackage>),
}

impl ServerMessage {
	pub fn encode(&self) -> [u8; 1000] {
		let mut slice = [0u8; 1000];
		bincode::encode_into_slice(self, &mut slice, bincode::config::standard()).unwrap();
		slice
	}
	pub fn decode(slice: &[u8]) -> Option<Self> {
		let o = bincode::decode_from_slice(slice, bincode::config::standard());
		match o {
		    Ok(r) => Some(r.0),
		    Err(_) => None,
		}
	}
}

#[derive(Encode, Decode, Debug)]
pub enum ClientMessage {
	Login,
	SetVelocity(NetIDType, MyVec2),
}

impl ClientMessage {
	pub fn encode(&self) -> [u8; 1000] {
		let mut slice = [0u8; 1000];
		bincode::encode_into_slice(self, &mut slice, bincode::config::standard()).unwrap();
		slice
	}
	pub fn decode(slice: &[u8]) -> Option<Self> {
		let o = bincode::decode_from_slice(slice, bincode::config::standard());
		match o {
		    Ok(r) => Some(r.0),
		    Err(_) => None,
		}
	}
}

// Define collision layers
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum Layer {
    #[default]
    Boundary,
    Ball,
}
