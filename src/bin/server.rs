use std::net::{SocketAddr, UdpSocket};
use std::collections::HashMap;
use dodgescrape2::*;

const ENEMY_RADIUS: f32 = 20.;

pub struct ServerSocket {
    pub socket: UdpSocket,
    pub buf: [u8; 1000],
}

impl ServerSocket {
    pub fn new(
        socket: UdpSocket,
    ) -> Self {
        Self {
            socket,
            buf: [0; 1000],
        }
    }
    pub fn send_to(&self, bytes: &[u8], addr: SocketAddr) -> bool {
        match self.socket.send_to(bytes, addr) {
            Ok(l) => l == bytes.len(),
            Err(_) => false,
        }
    }
}

#[derive(Resource)]
pub struct IncomingReceiver(crossbeam::channel::Receiver<(SocketAddr, ClientMessage)>);
#[derive(Resource)]
pub struct OutgoingSender(crossbeam::channel::Sender<(SocketAddr, ServerMessage)>);

fn main() {
    let (incoming_sender, incoming_receiver) = crossbeam::channel::unbounded::<(SocketAddr, ClientMessage)>();
    let (outgoing_sender, outgoing_receiver) = crossbeam::channel::unbounded::<(SocketAddr, ServerMessage)>();

    let network_thread = std::thread::spawn(move || {
        let socket = UdpSocket::bind("0.0.0.0:7878").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut server_socket = ServerSocket::new(socket);
        loop {
            // get from game
            while let Ok((addr, outgoing_package)) = outgoing_receiver.try_recv() {
                let bytes = outgoing_package.encode();
                server_socket.send_to(&bytes, addr);
            }

            // get from socket
            let ServerSocket { socket, buf } = &mut server_socket;

            while let Ok((len, addr)) = socket.recv_from(buf) {
                if let Some(client_message) = ClientMessage::decode(buf) {
                    incoming_sender.send((addr, client_message));
                }
            }
        }
    });

    App::new()
        .insert_resource(IncomingReceiver(incoming_receiver))
        .insert_resource(OutgoingSender(outgoing_sender))
        .insert_resource(Gravity::ZERO)
        .insert_resource(IDCounter(0))
        .insert_resource(EntityMap::default())
        .insert_resource(NetIDMap::default())
        .add_plugins(DefaultPlugins)
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, (setup, spawn_enemies))
        .add_systems(Update, (
            receive_messages,
            // apply_velocity_system,
            enemy_kill_system,
            broadcast_transform_updates,
            broadcast_enemy_spawns,
            broadcast_player_spawns))
        .run();
}

#[derive(Component)]
pub struct UpdateAddress {
    addr: SocketAddr,
}

#[derive(Resource, Default)]
struct NetIDMap(HashMap<Entity, NetIDType>);
#[derive(Resource, Default)]
struct EntityMap(HashMap<NetIDType, Entity>);

#[derive(Resource)]
struct IDCounter(pub NetIDType);

#[derive(Component)]
struct PendingSpawn;

fn receive_messages(
    incoming_receiver: Res<IncomingReceiver>,
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut id_counter: ResMut<IDCounter>,
    mut net_id_map: ResMut<NetIDMap>,
    mut entity_map: ResMut<EntityMap>,
    mut player_query: Query<&mut Velocity, With<Player>>,
) {
    while let Ok((addr, client_message)) = incoming_receiver.0.try_recv() {
        match client_message {
            ClientMessage::Login => {
                // spawn player
                let id = commands.spawn((
                    Transform::from_xyz(200., 0., 1.),
                    Player,
                    Alive(true),
                    Radius(20.),
                    LinearVelocity(Vec2::new(-200., 0.)),
                    Mesh2d(meshes.add(Circle::new(20.))),
                    MeshMaterial2d(materials.add(Color::srgb(0., 1., 0.))),
                    UpdateAddress {addr},
                    PendingSpawn,
                )).id();

                net_id_map.0.insert(id, id_counter.0);
                entity_map.0.insert(id_counter.0, id);
                outgoing_sender.0.send((addr, ServerMessage::Ok(id_counter.0)));

                id_counter.0 += 1;
            },
            ClientMessage::SetVelocity(player_net_id, velocity) => {
                let player_entity_option = entity_map.0.get(&player_net_id);
                let mut player_exists = false;
                match player_entity_option {
                    Some(player_entity) => {
                        let mut player_velocity_result = player_query.get_mut(*player_entity);
                        match player_velocity_result {
                            Ok(mut player_velocity) => {
                                player_exists = true;
                                player_velocity.0 = velocity.into();
                            },
                            Err(_) => {},
                        }
                    },
                    None => {},
                }
                if !player_exists {
                    entity_map.0.remove(&player_net_id);
                }
            },
        }
    }
}

fn broadcast_player_spawns(
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut id_counter: ResMut<IDCounter>,
    mut net_id_map: ResMut<NetIDMap>,
    mut entity_map: ResMut<EntityMap>,
    client_addresses: Query<(Entity, &UpdateAddress), With<PendingSpawn>>,
    player_query: Query<(Entity, &Transform, &Mesh2d, &LinearVelocity, &MeshMaterial2d<ColorMaterial>, &Player, &Alive, &Radius)>,
) {
    for (id, addr) in client_addresses.iter() {
        let mut entity_packages = Vec::<EntityPackage>::new();
        for (entity, transform, mesh2d, velocity, meshmaterial2d, player, alive, radius) in &player_query {
            let net_id = net_id_map.0.get(&entity).unwrap();
            entity_packages.push(EntityPackage { net_id: *net_id, components: vec![
                (*transform).into(),
                NetComponent::CircleMesh(radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial2d).unwrap().clone()).into(),
                (*player).into(),
                (*alive).into(),
                (*radius).into(),
            ] });
        }
        for chonky in entity_packages.chunks(10) {
            outgoing_sender.0.send((addr.addr, ServerMessage::SpawnEntities(chonky.to_vec())));
            commands.entity(id).remove::<PendingSpawn>();
        }
    }
}

fn broadcast_enemy_spawns(
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut id_counter: ResMut<IDCounter>,
    mut net_id_map: ResMut<NetIDMap>,
    mut entity_map: ResMut<EntityMap>,
    client_addresses: Query<(Entity, &UpdateAddress), With<PendingSpawn>>,
    enemy_query: Query<(Entity, &Transform, &Mesh2d, &LinearVelocity, &MeshMaterial2d<ColorMaterial>, &Enemy, &Radius)>,
) {
    for (id, addr) in client_addresses.iter() {
        let mut entity_packages = Vec::<EntityPackage>::new();
        for (entity, transform, mesh2d, velocity, meshmaterial2d, enemy, radius) in &enemy_query {
            let net_id = net_id_map.0.get(&entity).unwrap();
            entity_packages.push(EntityPackage { net_id: *net_id, components: vec![
                (*transform).into(),
                NetComponent::CircleMesh(radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial2d).unwrap().clone()).into(),
                (*enemy).into(),
                (*radius).into(),
            ] });
        }
        for chonky in entity_packages.chunks(10) {
            outgoing_sender.0.send((addr.addr, ServerMessage::SpawnEntities(chonky.to_vec())));
            commands.entity(id).remove::<PendingSpawn>();
        }
    }
}

const ENEMIES_PER_PACKAGE: usize = (1000. / std::mem::size_of::<EnemyPackage>() as f32).floor() as usize;
const PLAYERS_PER_PACKAGE: usize = (1000. / std::mem::size_of::<PlayerPackage>() as f32).floor() as usize;

fn broadcast_enemies(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    enemy_query: Query<(Entity, &Transform, &Radius), With<Enemy>>,
    mut net_id_map: ResMut<NetIDMap>,
) {
    const BROADCAST_RADIUS: f32 = 500.0;
    const RADIUS_SQUARED: f32 = BROADCAST_RADIUS * BROADCAST_RADIUS; // Avoid sqrt in distance checks

    // Process each client separately
    for (id, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;
        
        // Collect enemies within radius for this specific player
        let mut nearby_enemies: Vec<EnemyPackage> = enemy_query
            .iter()
            .filter_map(|(enemy_entity, enemy_transform, radius)| {
                let distance_squared = player_pos.distance_squared(enemy_transform.translation);
                
                if distance_squared <= RADIUS_SQUARED {
                    let net_id = net_id_map.0.get(&enemy_entity)?;
                    Some(EnemyPackage {
                        net_id: *net_id,
                        position: enemy_transform.translation.into(),
                        radius: radius.0,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Split into chunks and send
        for enemy_chunk in nearby_enemies.chunks(ENEMIES_PER_PACKAGE) {
            let message = ServerMessage::UpdateEnemies(enemy_chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message));
        }
    }
}

fn broadcast_transform_updates(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    transform_query: Query<(Entity, &Transform)>,
    mut net_id_map: ResMut<NetIDMap>,
) {
    const BROADCAST_RADIUS: f32 = 500.0;
    const RADIUS_SQUARED: f32 = BROADCAST_RADIUS * BROADCAST_RADIUS;

    for (id, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;
        
        let mut nearby: Vec<EntityPackage> = transform_query
            .iter()
            .filter_map(|(entity, transform)| {
                let distance_squared = player_pos.distance_squared(transform.translation);
                
                if distance_squared <= RADIUS_SQUARED {
                    let net_id = net_id_map.0.get(&entity)?;
                    Some(EntityPackage {
                        net_id: *net_id,
                        components: vec![NetComponent::Transform {
                            translation: transform.translation.into(),
                            rotation: Rotation2d(0.),
                            scale: Vec3::ONE.into()
                        }],
                    })
                } else {
                    None
                }
            })
            .collect();

        for chonky in nearby.chunks(10) {
            let message = ServerMessage::UpdateEntities(chonky.to_vec());
            outgoing_sender.0.send((addr.addr, message));
        }
    }
}

fn broadcast_players(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress)>,
    player_query: Query<(Entity, &Transform), With<Player>>,
    mut net_id_map: ResMut<NetIDMap>,
) {
    let player_package_vec_count = (player_query.iter().len() as f32 / PLAYERS_PER_PACKAGE as f32).ceil() as usize;
    let mut player_package_vec = Vec::<Vec<PlayerPackage>>::new();
    let mut player_packages: Vec<PlayerPackage> = Vec::with_capacity(PLAYERS_PER_PACKAGE);
    let mut counter = 0;
    for (player_entity, player_transform) in player_query {
        let net_id = net_id_map.0.get(&player_entity).unwrap();
        player_packages.push(PlayerPackage {
            net_id: *net_id,
            position: player_transform.translation.into(),
        });
        counter += 1;
        if counter >= PLAYERS_PER_PACKAGE {
            counter = 0;
            player_package_vec.push(player_packages);
            player_packages = Vec::with_capacity(PLAYERS_PER_PACKAGE);
        }
    }
    if player_packages.len() > 0 {
        player_package_vec.push(player_packages);
    }

    for player_packages in player_package_vec {
        let message = ServerMessage::UpdatePlayers(player_packages);

        for (id, addr) in client_addresses {
            outgoing_sender.0.send((addr.addr, message.clone()));
        }
    }
}

fn setup(
    mut commands: Commands,
) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Transform::from_xyz(0., 0., 0.),
        Tonemapping::TonyMcMapface,
        Bloom::default(),
        DebandDither::Enabled,
    ));
}

fn spawn_enemies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut id_counter: ResMut<IDCounter>,
    mut net_id_map: ResMut<NetIDMap>,
    mut entity_map: ResMut<EntityMap>,
) {
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
        // spawn vertical walls
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(thickness, half_boundary * 2.))),
            wall_material.clone(),
            Transform::from_xyz(pos, 0., 0.),
            RigidBody::Static,
            Collider::rectangle(thickness, half_boundary * 2.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
        // spawn horizontal walls
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(half_boundary * 2., thickness))),
            wall_material.clone(),
            Transform::from_xyz(0., pos, 0.),
            RigidBody::Static,
            Collider::rectangle(half_boundary * 2., thickness),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
    }

    for _ in 0..5000 {
        let velocity = LinearVelocity(random_velocity());
        let position = random_position(2000.);
        let material = MeshMaterial2d(materials.add(Color::srgb(
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
        )));

        // spawn enemy
        let id = commands.spawn((
            Transform::from_translation(position.extend(0.)),
            Mesh2d(meshes.add(Circle::new(ENEMY_RADIUS))),
            material,
            RigidBody::Dynamic,
            Collider::circle(ENEMY_RADIUS),
            velocity,
            CollisionLayers::new([Layer::Ball], [Layer::Boundary]),
            Restitution::new(1.0), // Perfect bounce (1.0 = 100% energy retained)
            Friction::ZERO.with_combine_rule(CoefficientCombine::Min), // Remove friction
            Enemy,
            Radius(ENEMY_RADIUS),
        )).id();

        net_id_map.0.insert(id, id_counter.0);
        entity_map.0.insert(id_counter.0, id);
        id_counter.0 += 1;
    }
}

// fn apply_velocity_system(
//     time: Res<Time>,
//     query: Query<(&mut Transform, &Velocity)>,
// ) {
//     let d = time.delta_secs();
//     for (mut transform, velocity) in query {
//         transform.translation += velocity.0.extend(0.) * d;
//     }
// }

fn enemy_kill_system(
    players: Query<(&mut Alive, &Transform, &Radius), With<Player>>,
    enemies: Query<(&Transform, &Radius), With<Enemy>>,
) {
    for (mut player_alive, player_pos, player_radius) in players {
        for (enemy_pos, enemy_radius) in enemies {
            let distance = player_pos.translation.distance(enemy_pos.translation);
            if distance - player_radius.0 - enemy_radius.0 <= 0. {
                player_alive.0 = false;
            }
        }
    }
}
