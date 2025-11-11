use std::net::{SocketAddr, UdpSocket};
use std::collections::{HashMap, HashSet};
use bevy::ecs::entity_disabling::Disabled;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy_royal::*;
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use bevy::{
    camera::visibility::RenderLayers, color::palettes::tailwind,
    input::mouse::AccumulatedMouseMotion, light::NotShadowCaster, prelude::*,
};
use std::f32::consts::FRAC_PI_2;
use std::f32::consts::PI;

const FOG_COLOR: Color = Color::srgb(0.15, 0.20, 0.30);

pub struct ClientSocket {
    pub target: String,
    pub socket: UdpSocket,
    pub buf: [u8; 1000],
}

impl ClientSocket {
    pub fn new(target: String) -> Self {
        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            buf: [0; 1000],
            target,
        }
    }
    pub fn send(&self, bytes: &[u8]) {
        self.socket.send_to(bytes, &self.target).unwrap();
    }
}

#[derive(Resource)]
pub struct IncomingReceiver(crossbeam::channel::Receiver<ServerMessage>);
#[derive(Resource)]
pub struct OutgoingSender(crossbeam::channel::Sender<ClientMessage>);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let server_address = args.get(1).cloned().unwrap_or("127.0.0.1:7878".to_string());

    let (incoming_sender, incoming_receiver) = crossbeam::channel::unbounded::<ServerMessage>();
    let (outgoing_sender, outgoing_receiver) = crossbeam::channel::unbounded::<ClientMessage>();

    let network_thread = std::thread::spawn(move || {
        let mut client_socket = ClientSocket::new(server_address);
        let mut delay_pool: Vec<(f32, ServerMessage)> = Vec::with_capacity(1000);
        let mut past = std::time::Instant::now();

        loop {
            // delta time
            let present = std::time::Instant::now();
            let delta_secs = present.duration_since(past).as_secs_f32();
            past = present;

            // get from game
            while let Ok(outgoing_package) = outgoing_receiver.try_recv() {
                let bytes = outgoing_package.encode();
                client_socket.send(&bytes);
            }

            // get from socket
            let ClientSocket { socket, buf, target: _ } = &mut client_socket;

            while let Ok((len, addr)) = socket.recv_from(buf) {
                if let Some(server_message) = ServerMessage::decode(buf) {
                    // incoming_sender.send(server_message);
                    delay_pool.push((0.0, server_message));
                }
                else {
                    println!("got something that couldnt be decoded");
                }
            }

            // go through delay pool
            let mut removed = Vec::<ServerMessage>::new();
            delay_pool.retain_mut(|(d, sm)| {
                *d -= delta_secs;
                if *d < 0. {
                    removed.push(sm.clone());
                    false
                }
                else {
                    true
                }
            });

            for server_message in removed {
                incoming_sender.send(server_message);
            }
        }
    });

    App::new()
        .insert_resource(IncomingReceiver(incoming_receiver))
        .insert_resource(OutgoingSender(outgoing_sender))
        .insert_resource(CursorPos(Vec2::ZERO))
        .insert_resource(EntityMap::default())
        .insert_resource(NetIDMap::default())
        .add_plugins(DefaultPlugins)
        // .add_plugins(EguiPlugin::default())
        // .add_plugins(WorldInspectorPlugin::new())
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, (setup, cursor_lock))
        .add_systems(Update, (
            receive_messages,
            cursor_position_system,
            rotate_player,
            player_movement_system,
        ))
        .run();
}

#[derive(Resource, Default)]
struct NetIDMap(HashMap<Entity, NetIDType>);
#[derive(Resource, Default)]
struct EntityMap(HashMap<NetIDType, Entity>);

#[derive(Component)]
struct Controlled;

fn setup(
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let login_message = ClientMessage::Login;
    outgoing_sender.0.send(login_message);

    let mut rng = rand::rng();
    // + Spawn static boundary colliders
    let half_boundary = 3000.0;
    let thickness = 10.0;
    let wall_material = MeshMaterial2d(materials.add(Color::srgb(
        rng.random_range(0.0..4.0),
        rng.random_range(0.0..4.0),
        rng.random_range(0.0..4.0),
    )));
    for &pos in &[-half_boundary, half_boundary] {
        // vertical walls
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(thickness, half_boundary * 2.))),
            wall_material.clone(),
            Transform::from_xyz(pos, 0., 0.),
            RigidBody::Static,
            Collider::cuboid(thickness, half_boundary * 2., 5.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
        // horizontal walls
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(half_boundary * 2., thickness))),
            wall_material.clone(),
            Transform::from_xyz(0., pos, 0.),
            RigidBody::Static,
            Collider::cuboid(half_boundary * 2., thickness, 5.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
    }

    commands.spawn((
        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("fiebigershof.glb"),
        )),
        Transform::from_xyz(0., 0., 0.)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(50.))
        ,
    ));
}

fn cursor_position_system(
    window: Single<&Window, With<PrimaryWindow>>,
    mut cursor: ResMut<CursorPos>,
) {
    let window_center = Vec2::new(window.width() / 2.0, window.height() / 2.0);

    if let Some(cursor_position) = window.cursor_position() {
        cursor.0 = (cursor_position - window_center) * Vec2::new(1., -1.); // relative to center
    }
}

fn player_movement_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut player_query: Query<(Entity, &mut Velocity, &Alive, &Transform), (With<Player>, With<Controlled>)>,
    outgoing_sender: Res<OutgoingSender>,
    net_id_map: Res<NetIDMap>,
) {
    let speed = 8.0;

    for (player_entity, mut velocity, alive, camera_transform) in player_query.iter_mut() {
        let (yaw, _pitch, _roll) = camera_transform.rotation.to_euler(EulerRot::ZXY);

        let yaw_rotation = Quat::from_axis_angle(Vec3::Z, yaw);

        let forward = yaw_rotation * Vec3::Y;
        let forward_2d = Vec2::new(forward.x, forward.y).normalize_or_zero();
        
        let right_2d = Vec2::new(-forward_2d.y, forward_2d.x);
        
        if alive.0 {
            let mut dir = Vec2::ZERO;

            if keyboard.pressed(KeyCode::KeyW) { dir += forward_2d; }
            if keyboard.pressed(KeyCode::KeyS) { dir -= forward_2d; }
            if keyboard.pressed(KeyCode::KeyA) { dir += right_2d; }
            if keyboard.pressed(KeyCode::KeyD) { dir -= right_2d; }

            if dir.length_squared() > 0.0 {
                dir = dir.normalize();
            }

            velocity.0 = (dir * speed).extend(0.);
        } else {
            velocity.0 = Vec3::ZERO;
        }

        let net_id = net_id_map.0.get(&player_entity).unwrap();
        outgoing_sender.0.send(ClientMessage::SetVelocity(*net_id, velocity.0.truncate().into()));
    }
}

#[derive(Debug, Component)]
struct Player;

#[derive(Debug, Component, Deref, DerefMut)]
struct CameraSensitivity(Vec2);

impl Default for CameraSensitivity {
    fn default() -> Self {
        Self(
            Vec2::new(0.003, 0.003),
        )
    }
}

fn rotate_player(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    player: Single<(Entity, &mut Transform, &CameraSensitivity), With<Controlled>>,
    outgoing_sender: Res<OutgoingSender>,
    net_id_map: Res<NetIDMap>,
) {
    let (player_entity, mut transform, camera_sensitivity) = player.into_inner();

    let delta = accumulated_mouse_motion.delta;

    if delta != Vec2::ZERO {
        let delta_yaw = -delta.x * camera_sensitivity.x;
        let delta_pitch = -delta.y * camera_sensitivity.y;

        let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::ZXY);
        let yaw = yaw + delta_yaw;

        const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;
        let pitch = (pitch + delta_pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);

        let new_rotation = Quat::from_euler(EulerRot::ZXY, yaw, pitch, roll);
        transform.rotation = new_rotation;

        let net_id = net_id_map.0.get(&player_entity).unwrap();
        outgoing_sender.0.send(ClientMessage::Rotation(*net_id, new_rotation.into()));
    }
}

fn receive_messages(
    incoming_receiver: Res<IncomingReceiver>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    mut entity_map: ResMut<EntityMap>,
    mut net_id_map: ResMut<NetIDMap>,
    mut transform_query: Query<(Entity, &mut Transform, Has<Controlled>)>,
) {

    loop {
        match incoming_receiver.0.try_recv() {
            Ok(server_message) => {
                match server_message {
                    ServerMessage::SpawnEntities(entity_packages) => {
                        for EntityPackage { net_id, components } in entity_packages {
                            if let Some(entity) = entity_map.0.get(&net_id) {
                                // already exists
                            }
                            else {
                                let mut entity = commands.spawn(( ));

                                for component in components {
                                    component.apply_to(&mut entity, &mut meshes, &mut standard_materials);
                                }

                                let id = entity.id();
                                entity_map.0.insert(net_id, id);
                                net_id_map.0.insert(id, net_id);
                            }
                        }
                    },
                    ServerMessage::UpdateEntities(entity_packages) => {
                        for EntityPackage { net_id, components } in entity_packages {
                            if let Some(entity) = entity_map.0.get(&net_id) {
                                if let Ok(mut entity_commands) = commands.get_entity(*entity) {
                                    for component in components {
                                        component.apply_to(&mut entity_commands, &mut meshes, &mut standard_materials);
                                    }
                                }
                            }
                        }
                    },
                    // receiv myself
                    ServerMessage::Ok(net_id) => {
                        println!("player was created successfully with id {:?}", net_id);

                        if !entity_map.0.contains_key(&net_id) {

                            commands.insert_resource(AmbientLight {
                                brightness: 1.,
                                ..Default::default()
                            });

                            // sun
                            // commands.spawn((
                            //     DirectionalLight {
                            //         illuminance: 320.0,
                            //         ..default()
                            //     },
                            //     Transform::from_xyz(0.0, 2.0, 0.0).with_rotation(Quat::from_rotation_x(-PI / 4.)),
                            // ));

                            commands.spawn((
                                Mesh3d(meshes.add(Plane3d::default().mesh().size(20000.0, 20000.0).subdivisions(10))),
                                MeshMaterial3d(standard_materials.add(Color::srgb(0.4, 0.5, 0.1))),
                                Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)).with_translation(Vec3::new(0., 0., 0.)),
                            ));
                            
                            // spawn player
                            let player_radius = 1.5;
                            let id = commands.spawn((
                                Mesh3d(meshes.add(Sphere::new(player_radius))),
                                Transform::default() ,
                                Velocity(Vec3::ZERO),
                                MeshMaterial3d(standard_materials.add(Color::srgb(0., 1., 0.))),
                                Player,
                                Alive(true),
                                Radius(player_radius),
                                Controlled,
                                CameraSensitivity::default(),
                                
                                children![
                                    (
                                        Camera3d::default(),
                                        Camera {
                                            clear_color: ClearColorConfig::Custom(FOG_COLOR),
                                            ..default()
                                        },
                                        DistanceFog {
                                            color: FOG_COLOR,
                                            falloff: FogFalloff::Linear {
                                                start: player_radius * 10.,
                                                end: player_radius * 100.,
                                            },
                                            ..default()
                                        },

                                        Projection::from(PerspectiveProjection {
                                            fov: 90.0_f32.to_radians(),
                                            ..default()
                                        }),
                                        Transform::from_xyz(0.0, - player_radius * 2.5, player_radius * 1.5).looking_to(Vec3::Y, Vec3::Z),

                                        Tonemapping::TonyMcMapface,
                                        Bloom::default(),
                                        DebandDither::Enabled,
                                    ),
                                    (
                                        Transform::from_xyz(0.0, - player_radius * 6.5, player_radius * 5.5).looking_to(Vec3::Y, Vec3::Z),
                                        SpotLight {
                                            shadows_enabled: true,
                                            intensity: player_radius * 10000000.,
                                            range: player_radius * 100.,
                                            shadow_depth_bias: 10.0,
                                            ..default()
                                        },
                                        // PointLight {
                                        //     shadows_enabled: true,
                                        //     intensity: 1000000000.,
                                        //     range: 3000.0,
                                        //     shadow_depth_bias: 10.0,
                                        //     ..default()
                                        // },
                                    ),
                                ],
                            )).id();

                            entity_map.0.insert(net_id, id);
                            net_id_map.0.insert(id, net_id);
                        }
                    },
                    ServerMessage::UpdatePositions(position_packages) => {
                        for position_package in position_packages {
                            if let Some(entity) = entity_map.0.get(&position_package.net_id) {
                                if let Ok((_, mut transform, controlled)) = transform_query.get_mut(*entity) {
                                    transform.translation = position_package.position.clone().into();
                                    if !controlled {
                                        transform.rotation = position_package.rotation.clone().into();
                                    }
                                }
                            }
                        }
                    },
                }
            }
            Err(e) => match e {
                crossbeam::channel::TryRecvError::Empty => break,
                crossbeam::channel::TryRecvError::Disconnected => break,
            },
        }
    }
}


fn cursor_lock(
    mut cursor_options: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    cursor_options.grab_mode = CursorGrabMode::Locked;
    cursor_options.visible = false;
}
