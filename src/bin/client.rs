use std::net::{SocketAddr, UdpSocket};
use std::collections::{HashMap, HashSet};
use bevy::ecs::entity_disabling::Disabled;
use bevy_royal::*;
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};

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
            player_movement_system,
            // update_camera_direction,
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
    cursor: Res<CursorPos>,
    player_query: Query<(Entity, &mut Velocity, &Alive), (With<Player>, With<Controlled>)>,
    outgoing_sender: Res<OutgoingSender>,
    mut net_id_map: Res<NetIDMap>,
) {
    for (player_entity, mut velocity, alive) in player_query {
        if alive.0 || true {
            let speed = 300.0; // units per second
            let length = cursor.0.length();
            let threshold = 200.;
            if length == 0. {
                continue;
            }
            let percentage = length / threshold;

            velocity.0 = cursor.0.normalize() * percentage * speed;
        }
        else {
            velocity.0 = Vec2::ZERO;
        }

        let net_id = net_id_map.0.get(&player_entity).unwrap();
        outgoing_sender.0.send(ClientMessage::SetVelocity(*net_id, velocity.0.into()));
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
                                        Tonemapping::TonyMcMapface,
                                        Bloom::default(),
                                        DebandDither::Enabled,
                                        Transform::from_xyz(0.0, -200., 2900.0)
                                            .looking_at(Vec3::new(0., 0., 0.), Vec3::Y)
                                        ,
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
