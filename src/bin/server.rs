use std::net::{SocketAddr, UdpSocket};
use std::collections::HashMap;
use bevy_royal::*;

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

#[derive(Component)]
pub struct LastBroadcast(f32);

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
        // .insert_resource(Gravity(Vec3::NEG_Z))
        .insert_resource(Gravity::ZERO)
        .insert_resource(IDCounter(0))
        .insert_resource(EntityMap::default())
        .insert_resource(NetIDMap::default())
        .add_plugins(DefaultPlugins)
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, (setup, spawn_enemies))
        .add_systems(Update, (
            receive_messages,
            apply_velocity_system,
            enemy_kill_system,
            broadcast_enemy_spawns,
            broadcast_player_spawns,
            broadcast_positions,
        ))
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
    mut player_query: Query<(&mut Velocity, &mut Transform), With<Player>>,
    client_addresses: Query<Entity, With<UpdateAddress>>,
) {
    while let Ok((addr, client_message)) = incoming_receiver.0.try_recv() {
        match client_message {
            ClientMessage::Login => {
                // spawn player
                let player_radius = 1.5;
                let id = commands.spawn((
                    Transform::from_xyz(0., 0., player_radius),
                    Player,
                    Alive(true),
                    Radius(player_radius),
                    Velocity(Vec3::ZERO),
                    // LinearVelocity(Vec2::new(-200., 0.)),
                    // RigidBody::Dynamic,
                    Mesh2d(meshes.add(Circle::new(player_radius))),
                    MeshMaterial2d(materials.add(Color::srgb(0., 1., 0.))),
                    UpdateAddress {addr},
                    PendingSpawn,
                )).id();

                net_id_map.0.insert(id, id_counter.0);
                entity_map.0.insert(id_counter.0, id);
                outgoing_sender.0.send((addr, ServerMessage::Ok(id_counter.0)));

                id_counter.0 += 1;

                // give all clients pending spawn
                for client in client_addresses {
                    commands.entity(client).insert(PendingSpawn);
                }
            },
            ClientMessage::SetVelocity(player_net_id, velocity) => {
                let player_entity_option = entity_map.0.get(&player_net_id);
                let mut player_exists = false;
                match player_entity_option {
                    Some(player_entity) => {
                        if let Ok((mut player_velocity, _)) = player_query.get_mut(*player_entity) {
                            player_exists = true;
                            player_velocity.0 = Into::<Vec2>::into(velocity).extend(0.);
                        }
                    },
                    None => {},
                }
                if !player_exists {
                    entity_map.0.remove(&player_net_id);
                }
            },
            ClientMessage::Rotation(player_net_id, rotation) => {
                let player_entity_option = entity_map.0.get(&player_net_id);
                let mut player_exists = false;
                match player_entity_option {
                    Some(player_entity) => {
                        if let Ok((_, mut player_transform)) = player_query.get_mut(*player_entity) {
                            player_exists = true;
                            player_transform.rotation = rotation.into();
                        }
                    },
                    None => {},
                }
                if !player_exists {
                    entity_map.0.remove(&player_net_id);
                }
            }
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
    player_query: Query<(Entity, &Transform, &Mesh2d, &Velocity, &MeshMaterial2d<ColorMaterial>, &Player, &Alive, &Radius)>,
) {
    for (id, addr) in client_addresses.iter() {
        // println!("client spawn");
        let mut entity_packages = Vec::<EntityPackage>::new();
        for (entity, transform, mesh2d, velocity, meshmaterial2d, player, alive, radius) in &player_query {
            println!("player broadcast");
            let net_id = net_id_map.0.get(&entity).unwrap();
            entity_packages.push(EntityPackage { net_id: *net_id, components: vec![
                (*transform).into(),
                NetComponent::Sphere(radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial2d).unwrap().clone()).into(),
                (*player).into(),
                (*alive).into(),
                (*radius).into(),
                NetComponent::SpotLight(radius.0),
            ] });
        }
        for chonky in entity_packages.chunks(2) {
            outgoing_sender.0.send((addr.addr, ServerMessage::SpawnEntities(chonky.to_vec())));
        }
        println!("sending player spawn");
        commands.entity(id).remove::<PendingSpawn>();
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
                NetComponent::Sphere(radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial2d).unwrap().clone()).into(),
                (*enemy).into(),
                (*radius).into(),
            ] });
        }
        for chonky in entity_packages.chunks(2) {
            outgoing_sender.0.send((addr.addr, ServerMessage::SpawnEntities(chonky.to_vec())));
            // commands.entity(id).remove::<PendingSpawn>();
        }
    }
}

const ENEMY_PACKAGES_PER_MESSAGE: usize = (1000. / std::mem::size_of::<EnemyPackage>() as f32).floor() as usize;
const POSITION_PACKAGES_PER_MESSAGE: usize = (1000. / std::mem::size_of::<PositionPackage>() as f32).floor() as usize;
const BROADCAST_RADIUS: f32 = 500.0;
const RADIUS_SQUARED: f32 = BROADCAST_RADIUS * BROADCAST_RADIUS;

fn broadcast_positions(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, Option<&mut LastBroadcast>)>,
    mut net_id_map: ResMut<NetIDMap>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (id, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;
        
        // Collect enemies within radius for this specific player
        let mut nearby_entities: Vec<PositionPackage> = query
            .iter_mut()
            .filter_map(|(entity, entity_transform, last_broadcast_option)| {
                let distance = player_pos.distance(entity_transform.translation);
                let net_id = net_id_map.0.get(&entity)?;
                let position_package = Some(PositionPackage {
                    net_id: *net_id,
                    position: entity_transform.translation.into(),
                    rotation: entity_transform.rotation.into(),
                });

                if let Some(mut last_broadcast) = last_broadcast_option {
                    last_broadcast.0 += delta_secs;
                    if last_broadcast.0 >= distance / 5000. {
                        last_broadcast.0 = 0.;
                        position_package
                    }
                    else {
                        None
                    }
                }
                else {
                    position_package
                }
            })
            .collect();

        // Split into chunks and send
        for enemy_chunk in nearby_entities.chunks(POSITION_PACKAGES_PER_MESSAGE) {
            let message = ServerMessage::UpdatePositions(enemy_chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message));
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
    let half_boundary = 1000.0;
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
            Collider::cuboid(thickness, half_boundary * 2., 5.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
        // spawn horizontal walls
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(half_boundary * 2., thickness))),
            wall_material.clone(),
            Transform::from_xyz(0., pos, 0.),
            RigidBody::Static,
            Collider::cuboid(half_boundary * 2., thickness, 5.),
            CollisionLayers::new([Layer::Boundary], [Layer::Ball]),
        ));
    }

    for _ in 0..2000 {
        let velocity = LinearVelocity(random_velocity(3., 9.));
        let position = random_position(half_boundary);
        let material = MeshMaterial2d(materials.add(Color::srgb(
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
        )));

        let enemy_radius = rng.random_range(1.0..2.0);

        // spawn enemy
        let id = commands.spawn((
            Transform::from_translation(position.extend(enemy_radius)),
            Mesh2d(meshes.add(Circle::new(enemy_radius))),
            material,
            RigidBody::Dynamic,
            Collider::sphere(enemy_radius),
            velocity,
            CollisionLayers::new([Layer::Ball], [Layer::Boundary]),
            Restitution::new(1.0), // Perfect bounce (1.0 = 100% energy retained)
            Friction::ZERO.with_combine_rule(CoefficientCombine::Min), // Remove friction
            Enemy,
            Radius(enemy_radius),
            LastBroadcast(0.),
        )).id();

        net_id_map.0.insert(id, id_counter.0);
        entity_map.0.insert(id_counter.0, id);
        id_counter.0 += 1;
    }
}

fn apply_velocity_system(
    time: Res<Time>,
    query: Query<(&mut Transform, &Velocity)>,
) {
    let d = time.delta_secs();
    for (mut transform, velocity) in query {
        transform.translation += velocity.0 * d;
    }
}

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
