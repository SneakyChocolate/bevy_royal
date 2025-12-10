pub use avian3d::prelude::*;
use bincode::{Decode, Encode};
pub use rand::Rng;
pub use bevy::window::PrimaryWindow;
pub use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    post_process::bloom::{Bloom, BloomCompositeMode},
    prelude::*,
};
use std::net::{SocketAddr, UdpSocket};

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
pub struct Health(pub f32);

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

pub const HALF_BOUNDARY: f32 = 500.0;

pub fn spawn_walls(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut rng = rand::rng();
    // + Spawn static boundary colliders
    let thickness = 10.0;
    let wall_material = MeshMaterial3d(materials.add(Color::srgb(
        0.,
        0.,
        0.,
    )));
    for &pos in &[-HALF_BOUNDARY, HALF_BOUNDARY] {
        // spawn vertical walls
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(thickness, HALF_BOUNDARY * 2., 200.))),
            wall_material.clone(),
            Transform::from_xyz(pos, 0., 0.),
            RigidBody::Static,
            Collider::cuboid(thickness, HALF_BOUNDARY * 2., 200.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        ));
        // spawn horizontal walls
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(HALF_BOUNDARY * 2., thickness, 200.))),
            wall_material.clone(),
            Transform::from_xyz(0., pos, 0.),
            RigidBody::Static,
            Collider::cuboid(HALF_BOUNDARY * 2., thickness, 200.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        ));
    }
}

#[derive(Encode, Decode, Debug, Clone, Copy)]
pub struct MyVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
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

#[derive(Encode, Decode, Debug, Clone, Copy, Default)]
pub struct MyQuat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
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
pub struct MyVec2 {
    pub x: f32,
    pub y: f32,
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
pub struct PositionPackage {
    pub net_id: NetIDType,
    pub position: MyVec3,
    pub rotation: MyQuat,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct PlayerLookPackage {
    pub net_id: NetIDType, // must be player
    pub rotation: MyQuat,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct VelocityPackage {
    pub net_id: NetIDType,
    pub velocity: MyVec3,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct HealthPackage {
    pub net_id: NetIDType,
    pub health: f32,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct EntityPackage {
    pub net_id: NetIDType,
    pub components: Vec<NetComponent>,
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct ServerMessage {
    // 0 means not reliable, otherwise put id so that it can be confirmed, in bevy just put 1 and the network thread will automatically assign
    pub reliable: usize,
    pub message: ServerMessageInner,
}

impl ServerMessage {
    pub fn ok(reliable: usize, net_id: NetIDType) -> Self {
        Self {
            reliable,
            message: ServerMessageInner::Ok(net_id),
        }
    }
    pub fn confirm(id: usize) -> Self {
        Self {
            reliable: 1,
            message: ServerMessageInner::Confirm(id),
        }
    }
    pub fn update_healths(packages: Vec<HealthPackage>) -> Self {
        Self {
            reliable: 1,
            message: ServerMessageInner::UpdateHealths(packages),
        }
    }
    pub fn spawn_entities(reliable: usize, packages: Vec<EntityPackage>) -> Self {
        Self {
            reliable,
            message: ServerMessageInner::SpawnEntities(packages),
        }
    }
    pub fn update_entities(reliable: usize, packages: Vec<EntityPackage>) -> Self {
        Self {
            reliable,
            message: ServerMessageInner::UpdateEntities(packages),
        }
    }
    pub fn update_positions(packages: Vec<PositionPackage>) -> Self {
        Self {
            reliable: 0,
            message: ServerMessageInner::UpdatePositions(packages),
        }
    }
    pub fn update_velocities(packages: Vec<VelocityPackage>) -> Self {
        Self {
            reliable: 0,
            message: ServerMessageInner::UpdateVelocities(packages),
        }
    }
    pub fn update_player_looks(packages: Vec<PlayerLookPackage>) -> Self {
        Self {
            reliable: 0,
            message: ServerMessageInner::UpdatePlayerLooks(packages),
        }
    }
}

#[derive(Encode, Decode, Debug, Clone)]
pub enum ServerMessageInner {
    Ok(NetIDType), // the id of the player so that it knows which id it is
    SpawnEntities(Vec<EntityPackage>),
    UpdateEntities(Vec<EntityPackage>),
    UpdatePositions(Vec<PositionPackage>),
    UpdatePlayerLooks(Vec<PlayerLookPackage>),
    UpdateVelocities(Vec<VelocityPackage>),
    UpdateHealths(Vec<HealthPackage>),
    Confirm(usize),
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
pub struct ClientMessage {
    pub reliable: usize,
    pub message: ClientMessageInner,
}

impl ClientMessage {
    pub fn login() -> Self {
        Self {
            reliable: 1,
            message: ClientMessageInner::Login,
        }
    }
    pub fn setvelocity(me: NetIDType, velocity: MyVec2) -> Self {
        Self {
            reliable: 0,
            message: ClientMessageInner::SetVelocity(me, velocity),
        }
    }
    pub fn jump(me: NetIDType) -> Self {
        Self {
            reliable: 1,
            message: ClientMessageInner::Jump(me),
        }
    }
    pub fn shoot(me: NetIDType, direction: MyVec3) -> Self {
        Self {
            reliable: 1,
            message: ClientMessageInner::Shoot(me, direction),
        }
    }
    pub fn rotation(me: NetIDType, rotation: MyQuat) -> Self {
        Self {
            reliable: 0,
            message: ClientMessageInner::Rotation(me, rotation),
        }
    }
    pub fn confirm(id: usize) -> Self {
        Self {
            reliable: 0,
            message: ClientMessageInner::Confirm(id),
        }
    }
}

#[derive(Encode, Decode, Debug)]
pub enum ClientMessageInner {
    Login,
    SetVelocity(NetIDType, MyVec2),
    Rotation(NetIDType, MyQuat),
    // confirm an important message from the server, so the server doesnt resend (tcp immitation)
    Confirm(usize),
    Jump(NetIDType),
    Shoot(NetIDType, MyVec3),
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
    Player,
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
    SphereCollider(f32),
    Capsule(f32, f32),
    CapsuleCollider(f32, f32),
    ColorMaterial {
        r: f32,
        g: f32,
        b: f32,
    },
    Health(f32),
    Player,
    Enemy,
    Radius(f32),
    SpotLight(f32),
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
            rotation: self.rotation.into(),
            scale: self.scale.into(),
        }
    }
}
impl Into<NetComponent> for StandardMaterial {
    fn into(self) -> NetComponent {
        let color = self.base_color.to_srgba();
        NetComponent::ColorMaterial {
            r: color.red,
            g: color.green,
            b: color.blue,
        }
    }
}
impl Into<NetComponent> for Health {
    fn into(self) -> NetComponent {
        NetComponent::Health(self.0)
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

#[derive(Component)]
pub struct PlayerLookAnchor(pub Entity);

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
                entity.insert(RigidBody::Dynamic);
            },
            NetComponent::Sphere(radius) => {
                entity.insert(Mesh3d(meshes.add(Sphere::new(*radius))));
            },
            NetComponent::SphereCollider(radius) => {
                entity.insert(Collider::sphere(*radius));
            },
            NetComponent::Capsule(radius, height) => {
                entity.insert(Mesh3d(meshes.add(Capsule3d::new(*radius, *height))));
            },
            NetComponent::CapsuleCollider(radius, height) => {
                entity.insert(Collider::capsule(*radius, *height));
            },
            NetComponent::ColorMaterial { r, g, b } => {
                entity.insert(MeshMaterial3d(materials.add(Color::srgb(*r, *g, *b))));
            },
            NetComponent::Health(v) => {
                entity.insert(Health(*v));
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
            NetComponent::SpotLight(player_radius) => {
                let mut look_anchor_id = None;

                entity.with_children(|parent| {
                    let id = parent.spawn((
                        Transform::default(),
                        children![(
                            Transform::from_xyz(0.0, 0., 0.).looking_to(Vec3::Y, Vec3::Z),
                            SpotLight {
                                shadows_enabled: true,
                                intensity: player_radius * 10000000.,
                                range: player_radius * 100.,
                                shadow_depth_bias: 0.1,
                                ..default()
                            },
                        )]
                    )).id();

                    look_anchor_id = Some(id);
                });
                entity.insert(PlayerLookAnchor(look_anchor_id.unwrap()));
            }
        }
    }
}

pub fn map_transform() -> Transform {
   Transform::from_xyz(0., 0., -35.)
        .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
        .with_scale(Vec3::splat(500.))
}
