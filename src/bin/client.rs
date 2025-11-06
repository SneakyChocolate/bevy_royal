use std::net::{SocketAddr, UdpSocket};
use std::collections::{HashMap, HashSet};
use bevy::ecs::entity_disabling::Disabled;
use bevy_royal::*;
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use bevy::{
    camera::visibility::RenderLayers, color::palettes::tailwind,
    input::mouse::AccumulatedMouseMotion, light::NotShadowCaster, prelude::*,
};
use std::f32::consts::FRAC_PI_2;

pub struct ClientSocket {
    pub socket: UdpSocket,
    pub buf: [u8; 1000],
}

impl ClientSocket {
    pub fn new() -> Self {
        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            buf: [0; 1000],
        }
    }
    pub fn send(&self, bytes: &[u8]) {
        self.socket.send_to(bytes, "127.0.0.1:7878").unwrap();
    }
}

#[derive(Resource)]
pub struct IncomingReceiver(crossbeam::channel::Receiver<ServerMessage>);
#[derive(Resource)]
pub struct OutgoingSender(crossbeam::channel::Sender<ClientMessage>);

fn main() {
    let (incoming_sender, incoming_receiver) = crossbeam::channel::unbounded::<ServerMessage>();
    let (outgoing_sender, outgoing_receiver) = crossbeam::channel::unbounded::<ClientMessage>();

    let network_thread = std::thread::spawn(move || {
        let mut client_socket = ClientSocket::new();
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
            let ClientSocket { socket, buf } = &mut client_socket;

            while let Ok((len, addr)) = socket.recv_from(buf) {
                if let Some(server_message) = ServerMessage::decode(buf) {
                    // incoming_sender.send(server_message);
                    delay_pool.push((0.0, server_message));
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
        // .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, setup)
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
struct JustUpdated;

#[derive(Component)]
struct Controlled;

fn setup(
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
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
            Collider::rectangle(thickness, half_boundary * 2.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
        // horizontal walls
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(half_boundary * 2., thickness))),
            wall_material.clone(),
            Transform::from_xyz(0., pos, 0.),
            RigidBody::Static,
            Collider::rectangle(half_boundary * 2., thickness),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
    }
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
    keyboard: Res<Input<KeyCode>>,
    player_query: Query<(Entity, &mut Velocity, &Alive), (With<Player>, With<Controlled>)>,
    camera_query: Query<&Transform, With<Camera>>,
    outgoing_sender: Res<OutgoingSender>,
    net_id_map: Res<NetIDMap>,
) {
    let camera_transform = camera_query.single();
    let speed = 300.0;

    // forward in 3D
    let forward = camera_transform.forward();
    // ignore z → convert to Vec2
    let forward_2d = Vec2::new(forward.x, forward.y).normalize_or_zero();

    // perpendicular right vector
    let right_2d = Vec2::new(-forward_2d.y, forward_2d.x);

    for (player_entity, mut velocity, alive) in player_query.iter_mut() {
        if alive.0 {
            let mut dir = Vec2::ZERO;

            if keyboard.pressed(KeyCode::W) { dir += forward_2d; }
            if keyboard.pressed(KeyCode::S) { dir -= forward_2d; }
            if keyboard.pressed(KeyCode::A) { dir -= right_2d; }
            if keyboard.pressed(KeyCode::D) { dir += right_2d; }

            if dir.length_squared() > 0.0 {
                dir = dir.normalize();
            }

            velocity.0 = dir * speed;
        } else {
            velocity.0 = Vec2::ZERO;
        }

        let net_id = net_id_map.0.get(&player_entity).unwrap();
        outgoing_sender.0.send(ClientMessage::SetVelocity(*net_id, velocity.0.into()));
    }
}

#[derive(Debug, Component)]
struct Player;

#[derive(Debug, Component, Deref, DerefMut)]
struct CameraSensitivity(Vec2);

impl Default for CameraSensitivity {
    fn default() -> Self {
        Self(
            // These factors are just arbitrary mouse sensitivity values.
            // It's often nicer to have a faster horizontal sensitivity than vertical.
            // We use a component for them so that we can make them user-configurable at runtime
            // for accessibility reasons.
            // It also allows you to inspect them in an editor if you `Reflect` the component.
            Vec2::new(0.003, 0.001),
        )
    }
}

fn rotate_player(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    player: Single<(&mut Transform, &CameraSensitivity)/* , With<Player> */>,
) {
    let (mut transform, camera_sensitivity) = player.into_inner();

    let delta = accumulated_mouse_motion.delta;

    if delta != Vec2::ZERO {
        // Note that we are not multiplying by delta_time here.
        // The reason is that for mouse movement, we already get the full movement that happened since the last frame.
        // This means that if we multiply by delta_time, we will get a smaller rotation than intended by the user.
        // This situation is reversed when reading e.g. analog input from a gamepad however, where the same rules
        // as for keyboard input apply. Such an input should be multiplied by delta_time to get the intended rotation
        // independent of the framerate.
        let delta_yaw = -delta.x * camera_sensitivity.x;
        let delta_pitch = -delta.y * camera_sensitivity.y;

        let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::ZXY);
        let yaw = yaw + delta_yaw;

        // If the pitch was ±¹⁄₂ π, the camera would look straight up or down.
        // When the user wants to move the camera back to the horizon, which way should the camera face?
        // The camera has no way of knowing what direction was "forward" before landing in that extreme position,
        // so the direction picked will for all intents and purposes be arbitrary.
        // Another issue is that for mathematical reasons, the yaw will effectively be flipped when the pitch is at the extremes.
        // To not run into these issues, we clamp the pitch to a safe range.
        const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;
        let pitch = (pitch + delta_pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);

        transform.rotation = Quat::from_euler(EulerRot::ZXY, yaw, pitch, roll);
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
    mut transform_query: Query<(Entity, &mut Transform)>,
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
                    ServerMessage::Ok(net_id) => {
                        println!("player was created successfully with id {:?}", net_id);

                        if !entity_map.0.contains_key(&net_id) {

                            commands.insert_resource(AmbientLight {
                                brightness: 10.,
                                ..Default::default()
                            });

                            commands.spawn((
                                Mesh3d(meshes.add(Plane3d::default().mesh().size(6000.0, 6000.0).subdivisions(1000))),
                                MeshMaterial3d(standard_materials.add(Color::srgb(1., 1., 1.))),
                                Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)).with_translation(Vec3::new(0., 0., 0.)),
                            ));
                            

                            let id = commands.spawn((
                                Mesh3d(meshes.add(Sphere::new(20.))),
                                Transform::default()
                                ,
                                Velocity(Vec2::new(0., 0.)),
                                MeshMaterial3d(standard_materials.add(Color::srgb(0., 1., 0.))),
                                Player,
                                Alive(true),
                                Radius(20.),
                                Controlled,
                                
                                children![
                                    (
                                        Camera3d::default(),
                                        Camera {
                                            clear_color: ClearColorConfig::Custom(Color::BLACK),
                                            ..default()
                                        },
                                        Projection::from(PerspectiveProjection {
                                            fov: 90.0_f32.to_radians(),
                                            ..default()
                                        }),
                                        Transform::from_xyz(0.0, 0., 0.0).looking_at(Vec3::new(0., 1., 0.), Vec3::Z),
                                        CameraSensitivity::default(),

                                        Tonemapping::TonyMcMapface,
                                        Bloom::default(),
                                        DebandDither::Enabled,
                                    ),
                                    (
                                        Transform::from_xyz(0.0, 0., 100.0),
                                        PointLight {
                                            shadows_enabled: true,
                                            intensity: 100000000.,
                                            range: 500.0,
                                            shadow_depth_bias: 10.0,
                                            ..default()
                                        },
                                    ),
                                ],
                            )).id();

                            entity_map.0.insert(net_id, id);
                            net_id_map.0.insert(id, net_id);
                        }
                    },
                    ServerMessage::UpdatePositions(position_packages) => {
                        for (entity, _) in &transform_query {
                            commands.entity(entity).remove::<JustUpdated>();
                        }
                        for position_package in position_packages {
                            if let Some(entity) = entity_map.0.get(&position_package.net_id) {
                                if let Ok((_, mut transform)) = transform_query.get_mut(*entity) {
                                    transform.translation = position_package.position.clone().into();
                                    commands.entity(*entity).insert(JustUpdated);
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
