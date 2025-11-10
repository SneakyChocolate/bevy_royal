pub use avian3d::prelude::*;
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

#[derive(Component, Clone, Copy)]
pub struct Velocity(pub Vec3);

#[derive(Component, Clone, Copy)]
pub struct Radius(pub f32);

#[derive(Component, Clone, Copy)]
pub struct Player;

#[derive(Component, Clone, Copy)]
pub struct Alive(pub bool);

#[derive(Component, Clone, Copy)]
pub struct Enemy;

pub fn random_velocity(min: f32, max: f32) -> Vec3 {
    let mut rng = rand::rng();
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let speed = rng.random_range(min..max);
    (Vec2::from_angle(angle) * speed).extend(0.)
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
pub struct MyQuat {
	x: f32,
	y: f32,
	z: f32,
	w: f32,
}

impl Into<Quat> for MyQuat {
    fn into(self) -> Quat {
    	Quat::from_xyzw(self.x, self.y, self.z, self.w)
    }
}

impl Into<MyQuat> for Quat {
    fn into(self) -> MyQuat {
    	MyQuat {
	        x: self.x,
	        y: self.y,
	        z: self.z,
	        w: self.w,
	    }
    }
}

#[derive(Encode, Decode, Debug, Clone, Copy)]
pub struct Rotation2d(pub f32);

 impl Into<Quat> for Rotation2d {
    fn into(self) -> Quat {
    	Quat::from_rotation_z(self.0.to_radians())
    }
}

#[derive(Encode, Decode, Debug, Clone, Copy)]
pub struct Rotation3d {
	horizontal: f32,
	vertical: f32,
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
pub struct PositionPackage {
	pub net_id: NetIDType,
	pub position: MyVec3,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct EntityPackage {
	pub net_id: NetIDType,
	pub components: Vec<NetComponent>,
}

#[derive(Encode, Decode, Debug, Clone)]
pub enum ServerMessage {
	Ok(NetIDType), // the id of the player so that it knows which id it is
	SpawnEntities(Vec<EntityPackage>),
	UpdateEntities(Vec<EntityPackage>),
	UpdatePositions(Vec<PositionPackage>),
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

#[derive(Encode, Decode, Debug, Clone)]
pub enum NetComponent {
	LinearVelocity(MyVec3),
	Transform {
		translation: MyVec3,
		rotation: MyQuat,
		scale: MyVec3,
	},
	Sphere(f32),
	ColorMaterial {
		r: f32,
		g: f32,
		b: f32,
	},
	Alive(bool),
	Player,
	Enemy,
	Radius(f32),
}

impl Into<NetComponent> for LinearVelocity {
    fn into(self) -> NetComponent {
    	NetComponent::LinearVelocity((*self).into())
    }
}
impl Into<NetComponent> for Velocity {
    fn into(self) -> NetComponent {
    	NetComponent::LinearVelocity((self.0).into())
    }
}
impl Into<NetComponent> for Transform {
    fn into(self) -> NetComponent {
    	NetComponent::Transform {
	        translation: self.translation.into(),
	        rotation: self.rotation.into(), // TODO TEMP
	        scale: self.scale.into(),
	    }
    }
}
impl Into<NetComponent> for ColorMaterial {
    fn into(self) -> NetComponent {
        let color = self.color.to_srgba();
        NetComponent::ColorMaterial {
            r: color.red,
            g: color.green,
            b: color.blue,
        }
    }
}
impl Into<NetComponent> for Alive {
    fn into(self) -> NetComponent {
    	NetComponent::Alive(self.0)
    }
}
impl Into<NetComponent> for Player {
    fn into(self) -> NetComponent {
    	NetComponent::Player
    }
}
impl Into<NetComponent> for Enemy {
    fn into(self) -> NetComponent {
    	NetComponent::Enemy
    }
}
impl Into<NetComponent> for Radius {
    fn into(self) -> NetComponent {
    	NetComponent::Radius(self.0)
    }
}


impl NetComponent {
    pub fn apply_to(
    	&self,
    	entity: &mut EntityCommands,
	    meshes: &mut ResMut<Assets<Mesh>>,
	    materials: &mut ResMut<Assets<StandardMaterial>>,
    ) {
        match self {
            NetComponent::Transform { translation, rotation, scale } => {
                entity.insert(Transform {
                    translation: (*translation).into(),
                    rotation: (*rotation).into(),
                    scale: (*scale).into(),
                });
            },
            NetComponent::LinearVelocity(v) => {
                entity.insert(LinearVelocity((*v).into()));
            },
            NetComponent::Sphere(radius) => {
            	entity.insert(Mesh3d(meshes.add(Sphere::new(*radius))));
            },
            NetComponent::ColorMaterial { r, g, b } => {
                entity.insert(MeshMaterial3d(materials.add(Color::srgb(*r, *g, *b))));
            },
            NetComponent::Alive(v) => {
                entity.insert(Alive(*v));
            },
            NetComponent::Player => {
                entity.insert(Player);
            },
            NetComponent::Enemy => {
                entity.insert(Enemy);
            },
            NetComponent::Radius(v) => {
                entity.insert(Radius(*v));
            },
        }
    }
}
