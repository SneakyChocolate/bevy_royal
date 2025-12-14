use std::net::UdpSocket;
use std::collections::HashMap;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy_royal::*;
// use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use bevy::{
    input::mouse::AccumulatedMouseMotion,
};
use std::f32::consts::FRAC_PI_2;

const FOG_COLOR: Color = Color::srgb(0.15, 0.20, 0.30);

#[derive(Resource)]
pub struct IncomingReceiver(crossbeam::channel::Receiver<ServerMessage>);

#[derive(Resource)]
pub struct OutgoingSender(crossbeam::channel::Sender<ClientMessage>);

#[derive(Resource, Default)]
struct NetIDMap(HashMap<Entity, NetIDType>);

#[derive(Resource, Default)]
struct EntityMap(HashMap<NetIDType, Entity>);

#[derive(Resource)]
struct PlayerMaterials {
    normal: Handle<StandardMaterial>,
    destroyed: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Past(RingBuf<TimeStamp>);

#[derive(Debug, Clone)]
struct TimeStamp {
    unix_time: u64,
    position: Vec3,
}

#[derive(Component)]
struct Controlled;

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

pub struct ClientSocket {
    pub target: String,
    pub socket: UdpSocket,
    pub buf: [u8; 1000],
}
struct ReliablePackage {
    bytes: [u8; 1000],
    last_send: std::time::Instant,
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
        self.socket.send_to(bytes, &self.target)/* .unwrap() */;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let server_address = args.get(1).cloned().unwrap_or("127.0.0.1:7878".to_string());

    let (incoming_sender, incoming_receiver) = crossbeam::channel::unbounded::<ServerMessage>();
    let (outgoing_sender, outgoing_receiver) = crossbeam::channel::unbounded::<ClientMessage>();

    let _network_thread = std::thread::spawn(move || {
        let mut client_socket = ClientSocket::new(server_address);
        let mut delay_pool: Vec<(f32, ServerMessage)> = Vec::with_capacity(1000);
        let mut past = std::time::Instant::now();

        let mut reliable_counter = 1;
        let mut reliable_packages = HashMap::<usize, ReliablePackage>::new();

        loop {

            // delta time
            let present = std::time::Instant::now();
            let delta_secs = present.duration_since(past).as_secs_f32();
            past = present;

            // resend all important messegaes if they werent confirmed yet
            let now = present;
            for (_, packet) in reliable_packages.iter_mut() {
                if now.duration_since(packet.last_send) > std::time::Duration::from_millis(300) {
                    client_socket.send(&packet.bytes);
                    packet.last_send = now;
                }
            }

            // get from game
            while let Ok(mut outgoing_package) = outgoing_receiver.try_recv() {
                if outgoing_package.reliable > 0 {
                    outgoing_package.reliable = reliable_counter;
                }
                let bytes = outgoing_package.encode();
                if outgoing_package.reliable > 0 {
                    reliable_packages.insert(reliable_counter, ReliablePackage {
                        bytes,
                        last_send: now,
                    });
                    reliable_counter += 1;
                }
                client_socket.send(&bytes);
            }

            // get from socket
            let ClientSocket { socket, buf, target: _ } = &mut client_socket;

            while let Ok((len, _addr)) = socket.recv_from(buf) {
                if let Some(ServerMessage {reliable, message: server_message}) = ServerMessage::decode(&buf[..len]) {
                    if let ServerMessageInner::Confirm(reliable) = &server_message {
                        reliable_packages.remove(reliable);
                    }
                    // incoming_sender.send(server_message);
                    delay_pool.push((0.0, ServerMessage {reliable, message: server_message}));
                }
                else {
                    println!("got something that couldnt be decoded");
                }
            }

            // go through delay pool
            let mut removed = Vec::<ServerMessage>::new();
            delay_pool.retain_mut(|(d, sm)| {
                *d += delta_secs;
                if *d >= 0.2 { // TODO do something cool with that delay
                    removed.push(sm.clone());
                    false
                }
                else {
                    true
                }
            });

            for server_message in removed {
                incoming_sender.send(server_message).unwrap();
            }

        }
    });

    App::new()
        .insert_resource(IncomingReceiver(incoming_receiver))
        .insert_resource(OutgoingSender(outgoing_sender))
        .insert_resource(CursorPos(Vec2::ZERO))
        .insert_resource(EntityMap::default())
        .insert_resource(NetIDMap::default())
        .insert_resource(Gravity::ZERO)
        // .insert_resource(Gravity(Vec3::NEG_Z))
        .add_plugins(DefaultPlugins)
        // .add_plugins(EguiPlugin::default())
        // .add_plugins(WorldInspectorPlugin::new())
        .add_plugins(UpdatePastPlugin)
        .add_plugins(UnixTimePlugin)
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, (
            setup,
            spawn_walls,
            cursor_lock,
            spawn_crosshair,
        ))
        .add_systems(Update, (
            receive_messages,
            cursor_position_system,
            rotate_player,
            player_movement_system,
            update_dead_color,
            player_shoot_system,
        ))
        .run();
}

fn setup(
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let map_transform = map_transform();

    let normal = materials.add(Color::srgb(0., 1., 0.));
    let destroyed = materials.add(Color::srgb(5.0, 0.0, 0.0));
    commands.insert_resource(PlayerMaterials { normal, destroyed });

    let login_message = ClientMessage::login();
    outgoing_sender.0.send(login_message).unwrap();

    commands.spawn((
        // ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        // CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        // RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("map_shooter12.glb"),
        )),
        map_transform.clone(),
    ));

    commands.spawn((
        // ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        // CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        // RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("map_trees1.glb"),
        )),
        map_transform.clone(),
    ));

    commands.spawn((
        // ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        // CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        // RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("house1.glb"),
        )),
        map_transform.clone(),
    ));

    commands.spawn((
        // ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        // CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        // RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("fiebigershof.glb"),
        )),
        Transform::from_xyz(20., -20., 0.)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(30.))
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
    rotation_query: Single<(&ChildOf, &Transform), With<CameraSensitivity>>,
    mut player_query: Query<(Entity, &mut LinearVelocity, &Health, &Transform), (With<Player>, With<Controlled>)>,
    outgoing_sender: Res<OutgoingSender>,
    net_id_map: Res<NetIDMap>,
) {
    let speed = 8.0;
    let camera_transform = rotation_query.1;

    for (player_entity, mut velocity, health, _transform) in player_query.iter_mut() {
        let (yaw, _pitch, _roll) = camera_transform.rotation.to_euler(EulerRot::ZXY);

        let yaw_rotation = Quat::from_axis_angle(Vec3::Z, yaw);

        let forward = yaw_rotation * Vec3::Y;
        let forward_2d = Vec2::new(forward.x, forward.y).normalize_or_zero();

        let right_2d = Vec2::new(-forward_2d.y, forward_2d.x);
        let net_id = net_id_map.0.get(&player_entity).unwrap();

        if health.0 != 0. {
            let mut dir = Vec2::ZERO;

            if keyboard.pressed(KeyCode::KeyW) { dir += forward_2d; }
            if keyboard.pressed(KeyCode::KeyS) { dir -= forward_2d; }
            if keyboard.pressed(KeyCode::KeyA) { dir += right_2d; }
            if keyboard.pressed(KeyCode::KeyD) { dir -= right_2d; }
            if keyboard.just_pressed(KeyCode::Space) {
                outgoing_sender.0.send(ClientMessage::jump(*net_id)).unwrap();
            }

            if dir.length_squared() > 0.0 {
                dir = dir.normalize();
            }

            velocity.0 = (dir * speed).extend(0.);
        } else {
            velocity.0 = Vec3::ZERO;
        }

        outgoing_sender.0.send(ClientMessage::setvelocity(*net_id, velocity.0.truncate().into())).unwrap();
    }
}

fn rotate_player(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    player: Single<(&ChildOf, &mut Transform, &CameraSensitivity)>,
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

        let net_id = net_id_map.0.get(&player_entity.0).expect("no entity found in map");
        outgoing_sender.0.send(ClientMessage::rotation(*net_id, new_rotation.into())).unwrap();
    }
}

fn player_shoot_system(
    mouse: Res<ButtonInput<MouseButton>>,
    rotation_query: Single<(&ChildOf, &Transform), With<CameraSensitivity>>,
    mut player_query: Query<(Entity, &mut LinearVelocity, &Health, &Transform), (With<Player>, With<Controlled>)>,
    outgoing_sender: Res<OutgoingSender>,
    net_id_map: Res<NetIDMap>,

    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let camera_transform = rotation_query.1;
    let shot_direction = camera_transform.rotation * Vec3::Y;

    for (player_entity, mut velocity, health, transform) in player_query.iter_mut() {
        if health.0 == 0. {
            continue;
        }
        let net_id = net_id_map.0.get(&player_entity).unwrap();

        let ray_origin = transform.translation;
        let ray_dir = shot_direction.normalize();
        let ray_length = 10.0;

        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.05, 0.05, ray_length).mesh())),
            MeshMaterial3d(standard_materials.add(Color::srgb(1., 0., 0.))),
            Transform::from_translation(ray_origin + ray_dir * ray_length / 2.0)
                .looking_to(ray_dir, Vec3::Z),
        ));

        outgoing_sender.0.send(ClientMessage::shoot(*net_id, ( shot_direction ).into())).unwrap();
    }
}

fn receive_messages(
    incoming_receiver: Res<IncomingReceiver>,
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    mut entity_map: ResMut<EntityMap>,
    mut net_id_map: ResMut<NetIDMap>,
    mut transform_query: Query<(Entity, &mut Transform, Has<Controlled>, Option<&Past>)>,
    mut anchor_query: Query<(Entity, &PlayerLookAnchor)>,
    mut velocity_query: Query<(Entity, &mut LinearVelocity, Has<Controlled>)>,
    mut health_query: Query<(Entity, &mut Health)>,
    unix_time: Res<UnixTime>,
) {

    loop {
        match incoming_receiver.0.try_recv() {
            Ok(ServerMessage {
                reliable,
                message,
            }) => {

                if reliable > 0 {
                    outgoing_sender.0.send(ClientMessage::confirm(reliable));
                }
                match message {

                    ServerMessageInner::Confirm(_) => {
                    },

                    ServerMessageInner::SpawnEntities(entity_packages) => {
                        for EntityPackage { net_id, components } in entity_packages {
                            if let Some(_) = entity_map.0.get(&net_id) {
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
                    ServerMessageInner::UpdateEntities(entity_packages) => {
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
                    ServerMessageInner::Ok(net_id) => {
                        if !entity_map.0.contains_key(&net_id) {
                            println!("player was created successfully with id {:?}", net_id);

                            commands.insert_resource(AmbientLight {
                                brightness: 1.,
                                ..Default::default()
                            });

                            // sun
                            commands.spawn((
                                DirectionalLight {
                                    illuminance: 320.0,
                                    ..default()
                                },
                                Transform::from_xyz(0.0, 2.0, 0.0).with_rotation(Quat::from_rotation_x(-std::f32::consts::PI / 4.)),
                            ));

                            let player_radius = 1.5;

                            // spawn player

                            let look_anchor_entity = commands.spawn((
                                // spin this, PlayerLookAnchor points here
                                Transform::from_xyz(0., 0.0, player_radius * 0.5),
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
                                                start: player_radius * 100.,
                                                end: player_radius * 300.,
                                            },
                                            ..default()
                                        },

                                        Projection::from(PerspectiveProjection {
                                            fov: 90.0_f32.to_radians(),
                                            ..default()
                                        }),
                                        Transform::from_xyz(0.0, 0., 0.).looking_to(Vec3::Y, Vec3::Z),

                                        Tonemapping::TonyMcMapface,
                                        Bloom::default(),
                                        DebandDither::Enabled,
                                    ),

                                    (
                                        Transform::from_xyz(0.0, 0., 0.).looking_to(Vec3::Y, Vec3::Z),
                                        SpotLight {
                                            shadows_enabled: true,
                                            intensity: player_radius * 10000000.,
                                            range: player_radius * 100.,
                                            shadow_depth_bias: 0.1,
                                            ..default()
                                        },
                                    ),

                                ],
                            )).id();

                            let id = commands.spawn((
                                Transform::default(),
                                Player,
                                PlayerLookAnchor(look_anchor_entity),
                                Health(100.),
                                Radius(player_radius),
                                Controlled,
                                Past(RingBuf::new(10)),

                                LinearVelocity(Vec3::ZERO),
                                RigidBody::Dynamic,
                                CollisionLayers::new([Layer::Player], [Layer::Boundary]),
                                Collider::capsule(0.4, player_radius),
                                LockedAxes::ROTATION_LOCKED,
                                SweptCcd::default(),

                                children![

                                    (
                                        MeshMaterial3d(standard_materials.add(Color::srgb(0., 1., 0.))),
                                        Mesh3d(meshes.add(Capsule3d::new(0.4, player_radius))),
                                        Transform::from_rotation(Quat::from_rotation_x(90_f32.to_radians())),
                                    ),

                                ],
                            )).id();

                            commands.entity(id).add_child(look_anchor_entity);

                            entity_map.0.insert(net_id, id);
                            net_id_map.0.insert(id, net_id);
                        }
                    },

                    ServerMessageInner::UpdatePlayerLooks(packages) => {
                        // FIXME its setting the rotation but nothing visible
                        for package in packages {
                            if let Some(player_entity) = entity_map.0.get(&package.net_id) {
                                let anchor = if let Ok(anchor) = anchor_query.get(*player_entity) { anchor } else {continue;};
                                let entity = anchor.0;
                                if let Ok((_, mut transform, controlled, _)) = transform_query.get_mut(entity) {
                                    if !controlled {
                                        transform.rotation = package.rotation.clone().into();
                                    }
                                }
                            }
                        }
                    },

                    ServerMessageInner::UpdatePositions{unix_time: message_unix_time, packages} => {
                        for position_package in packages {
                            if let Some(entity) = entity_map.0.get(&position_package.net_id) {
                                if let Ok((_, mut transform, controlled, past_option)) = transform_query.get_mut(*entity) {
                                    // if the entity has past storage (which is only the client itself because of client prediction)
                                    if let Some(past) = past_option {
                                        // get the lower and upper timestamps from the past, interpolate the position to the received message timestamp and calculate the difference between that position and the position in the received message. that is the ammount that the past was wrongly calculated and needs to be fixed now (add diff to current pos)
                                        let ( lower_index, lower_time_stamp ) = past.0
                                            .iter()
                                            .enumerate()
                                            .find(|(i, time_stamp)| {time_stamp.unix_time < message_unix_time})
                                            .unwrap()
                                            .clone()
                                        ;

                                        // there can be a case where the past doesnt have a upper timestamp. if so, just take the present and interpolate between lower timestamp and present
                                        let upper_time_stamp = if lower_index < 0 {
                                            past.0.get(lower_index + 1).unwrap().clone()
                                        }
                                        else {
                                            TimeStamp {
                                                unix_time: unix_time.0,
                                                position: transform.translation,
                                            }
                                        };
                                    }

                                    transform.translation = position_package.position.clone().into();
                                    if !controlled {
                                        transform.rotation = position_package.rotation.clone().into();
                                    }
                                }
                            }
                        }
                    },

                    ServerMessageInner::UpdateVelocities(velocity_packages) => {
                        for package in velocity_packages {
                            if let Some(entity) = entity_map.0.get(&package.net_id) {
                                if let Ok((_, mut velocity, controlled)) = velocity_query.get_mut(*entity) {
                                    if !controlled {
                                        velocity.0 = package.velocity.into();
                                    }
                                }
                            }
                        }
                    },

                    ServerMessageInner::UpdateHealths(packages) => {
                        for package in packages {
                            if let Some(entity) = entity_map.0.get(&package.net_id) {
                                if let Ok((_, mut health)) = health_query.get_mut(*entity) {
                                    health.0 = package.health;
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

// TODO figure out why this only works without the player componnent
fn update_dead_color(
    mut materials: ResMut<Assets<StandardMaterial>>,
    health_q: Query<(Entity, &Health, &Children), Changed<Health>>,
    material_q: Query<&MeshMaterial3d<StandardMaterial>>,
) {
    for (entity, health, children) in &health_q {
        let health_percent = (health.0 / 100.0).clamp(0.0, 1.0);
        let color = Color::srgb(
            1.0 - health_percent,
            health_percent,
            0.0,
        );

        if let Ok(mat_handle) = material_q.get(entity) {
            if let Some(material) = materials.get_mut(mat_handle.0.id()) {
                material.base_color = color;
            }
        }

        for child in children.iter() {
            if let Ok(mat_handle) = material_q.get(child) {
                if let Some(material) = materials.get_mut(mat_handle.0.id()) {
                    material.base_color = color;
                }
            }
        }
    }
}

// ai generated by claude for testing
fn spawn_crosshair(mut commands: Commands) {
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            position_type: PositionType::Absolute,
            ..default()
        })
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Px(20.0),
                        height: Val::Px(20.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                ))
                .with_children(|crosshair| {
                    // Horizontal line
                    crosshair.spawn((
                        Node {
                            width: Val::Px(20.0),
                            height: Val::Px(2.0),
                            position_type: PositionType::Absolute,
                            top: Val::Px(9.0),
                            left: Val::Px(0.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                    ));

                    // Vertical line
                    crosshair.spawn((
                        Node {
                            width: Val::Px(2.0),
                            height: Val::Px(20.0),
                            position_type: PositionType::Absolute,
                            top: Val::Px(0.0),
                            left: Val::Px(9.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
                    ));
                });
        });
}

#[derive(Resource)]
struct LastUpdatePast(f32);

struct UpdatePastPlugin;
impl Plugin for UpdatePastPlugin {
    fn build(&self, app: &mut App) {
        app
            .insert_resource(LastUpdatePast(0.))
            .add_systems(Update, update_past)
        ;
    }
}

fn update_past(
    mut past_q: Query<( &mut Past, &Transform )>,
    mut last_update_past: ResMut<LastUpdatePast>,
    time: Res<Time>,
    unix_time: Res<UnixTime>,
) {
    last_update_past.0 += time.delta_secs();
    if last_update_past.0 < 0.1 {return;}
    last_update_past.0 = 0.;

    for (mut past, transform) in &mut past_q {
        past.0.push(TimeStamp {
            unix_time: unix_time.0,
            position: transform.translation.clone(),
        });
    }
}
