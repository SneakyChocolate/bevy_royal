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

struct ReliablePackage {
    bytes: [u8; 1000],
    addr: SocketAddr,
    last_send: std::time::Instant,
}

fn main() {

    let (incoming_sender, incoming_receiver) = crossbeam::channel::unbounded::<(SocketAddr, ClientMessage)>();
    let (outgoing_sender, outgoing_receiver) = crossbeam::channel::unbounded::<(SocketAddr, ServerMessage)>();

    let _network_thread = std::thread::spawn(move || {
        let socket = UdpSocket::bind("0.0.0.0:7878").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut server_socket = ServerSocket::new(socket);

        let mut reliable_counter = 1;
        let mut reliable_packages = HashMap::<usize, ReliablePackage>::new();

        loop {
            // resend all important messegaes if they werent confirmed yet
            let now = std::time::Instant::now();
            for (_, packet) in reliable_packages.iter_mut() {
                if now.duration_since(packet.last_send) > std::time::Duration::from_millis(300) {
                    server_socket.send_to(&packet.bytes, packet.addr);
                    packet.last_send = now;
                }
            }

            // get from game
            while let Ok((addr, mut outgoing_package)) = outgoing_receiver.try_recv() {
                if outgoing_package.reliable > 0 {
                    outgoing_package.reliable = reliable_counter;
                }
                let bytes = outgoing_package.encode();
                if outgoing_package.reliable > 0 {
                    reliable_packages.insert(reliable_counter, ReliablePackage {
                        bytes,
                        addr,
                        last_send: now,
                    });
                    reliable_counter += 1;
                }
                server_socket.send_to(&bytes, addr);
            }

            // get from socket
            let ServerSocket { socket, buf } = &mut server_socket;

            while let Ok((len, addr)) = socket.recv_from(buf) {
                if let Some(ClientMessage {reliable, message: client_message}) = ClientMessage::decode(&buf[..len]) {
                    if let ClientMessageInner::Confirm(reliable) = &client_message {
                        reliable_packages.remove(reliable);
                    }
                    incoming_sender.send((addr, ClientMessage {reliable, message: client_message})).unwrap();
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    App::new()
        .insert_resource(IncomingReceiver(incoming_receiver))
        .insert_resource(OutgoingSender(outgoing_sender))
        .insert_resource(Gravity(Vec3::NEG_Z * 9.))
        // .insert_resource(Gravity::ZERO)
        .insert_resource(IDCounter(0))
        .insert_resource(EntityMap::default())
        .insert_resource(NetIDMap::default())
        .add_plugins(DefaultPlugins)
        .add_plugins(PhysicsPlugins::default())
        .add_systems(Startup, (
            setup,
            spawn_enemies,
            spawn_walls,
        ))
        .add_systems(Update, (
            receive_messages,
            apply_velocity_system,
            enemy_kill_system,
            broadcast_enemy_spawns,
            broadcast_player_spawns,
            broadcast_positions,
            broadcast_velocities,
        ))
        .run();
}

#[derive(Resource)]
pub struct IncomingReceiver(crossbeam::channel::Receiver<(SocketAddr, ClientMessage)>);

#[derive(Resource)]
pub struct OutgoingSender(crossbeam::channel::Sender<(SocketAddr, ServerMessage)>);

#[derive(Resource, Default)]
struct NetIDMap(HashMap<Entity, NetIDType>);

#[derive(Resource, Default)]
struct EntityMap(HashMap<NetIDType, Entity>);

#[derive(Resource)]
struct IDCounter(pub NetIDType);

#[derive(Component)]
pub struct LastBroadcast(pub HashMap<SocketAddr, f32>);

#[derive(Component)]
struct PendingSpawn;

#[derive(Component)]
pub struct UpdateAddress {
    addr: SocketAddr,
}

type PlayerVelocityType = LinearVelocity;

fn receive_messages(
    incoming_receiver: Res<IncomingReceiver>,
    outgoing_sender: Res<OutgoingSender>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut id_counter: ResMut<IDCounter>,
    mut net_id_map: ResMut<NetIDMap>,
    mut entity_map: ResMut<EntityMap>,
    mut player_query: Query<(&mut PlayerVelocityType, &mut Transform), With<Player>>,
    client_addresses: Query<Entity, With<UpdateAddress>>,
) {
    while let Ok((addr, ClientMessage {reliable, message: client_message})) = incoming_receiver.0.try_recv() {
        match client_message {
            ClientMessageInner::Confirm(_) => {},

            ClientMessageInner::Login => {
                // spawn player
                let player_radius = 1.5;
                let id = commands.spawn((
                    Transform::from_xyz(0., 0., player_radius + 10.),
                    Player,
                    Alive(true),
                    Radius(player_radius),
                    // PlayerVelocityType::new(Vec3::ZERO),
                    LinearVelocity(Vec3::new(10., -10., 0.)),
                    RigidBody::Dynamic,
                    CollisionLayers::new([Layer::Player], [Layer::Boundary]),
                    Collider::sphere(player_radius),

                    Mesh3d(meshes.add(Sphere::new(player_radius))),
                    MeshMaterial3d(materials.add(Color::srgb(0., 1., 0.))),
                    UpdateAddress {addr},
                    PendingSpawn,
                    LastBroadcast(HashMap::new()),
                )).id();

                net_id_map.0.insert(id, id_counter.0);
                entity_map.0.insert(id_counter.0, id);
                outgoing_sender.0.send((addr, ServerMessage::ok(1, id_counter.0))).unwrap();

                id_counter.0 += 1;

                // give all clients pending spawn
                for client in client_addresses {
                    commands.entity(client).insert(PendingSpawn);
                }
            },

            ClientMessageInner::SetVelocity(player_net_id, velocity) => {
                let player_entity_option = entity_map.0.get(&player_net_id);
                let mut player_exists = false;
                match player_entity_option {
                    Some(player_entity) => {
                        if let Ok((mut player_velocity, _)) = player_query.get_mut(*player_entity) {
                            player_exists = true;
                            player_velocity.0.x = velocity.x;
                            player_velocity.0.y = velocity.y;
                            // player_velocity.0 = Into::<Vec2>::into(velocity).extend(0.);
                        }
                    },
                    None => {},
                }
                if !player_exists {
                    entity_map.0.remove(&player_net_id);
                }
            },

            ClientMessageInner::Rotation(player_net_id, rotation) => {
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
    materials: ResMut<Assets<StandardMaterial>>,
    net_id_map: ResMut<NetIDMap>,
    client_addresses: Query<(Entity, &UpdateAddress), With<PendingSpawn>>,
    player_query: Query<(Entity, &Transform, &PlayerVelocityType, &MeshMaterial3d<StandardMaterial>, &Player, &Alive, &Radius)>,
) {
    for (id, addr) in client_addresses.iter() {
        // println!("client spawn");
        let mut entity_packages = Vec::<EntityPackage>::new();
        for (entity, transform, velocity, meshmaterial3d, player, alive, radius) in &player_query {
            println!("player broadcast");
            let net_id = net_id_map.0.get(&entity).unwrap();
            entity_packages.push(EntityPackage { net_id: *net_id, components: vec![
                (*transform).into(),
                NetComponent::Sphere(radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial3d).unwrap().clone()).into(),
                (*player).into(),
                (*alive).into(),
                (*radius).into(),
                NetComponent::SpotLight(radius.0),
            ] });
        }
        for chonky in entity_packages.chunks(2) {
            outgoing_sender.0.send((addr.addr, ServerMessage::spawn_entities(1, chonky.to_vec()))).unwrap();
        }
        println!("sending player spawn");
        commands.entity(id).remove::<PendingSpawn>();
    }
}

fn broadcast_enemy_spawns(
    outgoing_sender: Res<OutgoingSender>,
    materials: ResMut<Assets<StandardMaterial>>,
    net_id_map: ResMut<NetIDMap>,
    client_addresses: Query<(Entity, &UpdateAddress), With<PendingSpawn>>,
    enemy_query: Query<(Entity, &Transform, &LinearVelocity, &MeshMaterial3d<StandardMaterial>, &Enemy, &Radius)>,
) {
    for (_, addr) in client_addresses.iter() {
        let mut entity_packages = Vec::<EntityPackage>::new();
        for (entity, transform, velocity, meshmaterial3d, enemy, radius) in &enemy_query {
            let net_id = net_id_map.0.get(&entity).unwrap();
            entity_packages.push(EntityPackage { net_id: *net_id, components: vec![
                (*transform).into(),
                NetComponent::Sphere(radius.0),
                NetComponent::SphereCollider(radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial3d).unwrap().clone()).into(),
                (*enemy).into(),
                (*radius).into(),
            ] });
        }
        for chonky in entity_packages.chunks(5) {
            outgoing_sender.0.send((addr.addr, ServerMessage::spawn_entities(1, chonky.to_vec()))).unwrap();
            // commands.entity(id).remove::<PendingSpawn>();
        }
    }
}

const POSITION_PACKAGES_PER_MESSAGE: usize = (1000. / std::mem::size_of::<PositionPackage>() as f32).floor() as usize;
const VELOCITY_PACKAGES_PER_MESSAGE: usize = (1000. / std::mem::size_of::<VelocityPackage>() as f32).floor() as usize;

fn update_per_distance(
    addr: SocketAddr,
    delta_secs: f32,
    last_broadcast_option: Option<Mut<LastBroadcast>>,
    distance: f32,
) -> bool {
    if let Some(mut last_broadcast) = last_broadcast_option {
        let mut lb = last_broadcast.0.entry(addr).or_insert(0.);
        *lb += delta_secs;
        if *lb >= distance / 200. {
            *lb = 0.0;
            true
        }
        else {
            false
        }
    }
    else {
        false
    }
}

fn broadcast_positions(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, Option<&mut LastBroadcast>)>,
    net_id_map: ResMut<NetIDMap>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (_entity, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;

        // Collect enemies within radius for this specific player
        let nearby_entities: Vec<PositionPackage> = query
            .iter_mut()
            .filter_map(|(entity, entity_transform, last_broadcast_option)| {
                let distance = player_pos.distance(entity_transform.translation);
                let net_id = net_id_map.0.get(&entity)?;

                if update_per_distance(addr.addr, delta_secs, last_broadcast_option, distance) {
                    Some(PositionPackage {
                        net_id: *net_id,
                        position: entity_transform.translation.into(),
                        rotation: entity_transform.rotation.into(),
                    })
                }
                else {
                    None
                }
            })
            .collect();

        // Split into chunks and send
        for chunk in nearby_entities.chunks(POSITION_PACKAGES_PER_MESSAGE) {
            let message = ServerMessage::update_positions(chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message)).unwrap();
        }
    }
}

fn broadcast_velocities(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, &LinearVelocity, Option<&mut LastBroadcast>)>,
    net_id_map: ResMut<NetIDMap>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (_entity, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;

        // Collect enemies within radius for this specific player
        let nearby_entities: Vec<VelocityPackage> = query
            .iter_mut()
            .filter_map(|(entity, entity_transform, entity_velocity, last_broadcast_option)| {
                let distance = player_pos.distance(entity_transform.translation);
                let net_id = net_id_map.0.get(&entity)?;

                if update_per_distance(addr.addr, delta_secs, last_broadcast_option, distance) {
                    Some(VelocityPackage {
                        net_id: *net_id,
                        velocity: entity_velocity.0.into(),
                    })
                }
                else {
                    None
                }
            })
            .collect();

        // Split into chunks and send
        for chunk in nearby_entities.chunks(VELOCITY_PACKAGES_PER_MESSAGE) {
            let message = ServerMessage::update_velocities(chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message)).unwrap();
        }
    }
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(AmbientLight {
        brightness: 50.,
        ..Default::default()
    });

    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Transform::from_xyz(0., 0., 500.).looking_at(Vec3::ZERO, Vec3::Y),
        Tonemapping::TonyMcMapface,
        Bloom::default(),
        DebandDither::Enabled,
    ));

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(2000.0, 2000.0).subdivisions(10))),
        MeshMaterial3d(standard_materials.add(Color::srgb(0.4, 0.5, 0.1))),
        Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_translation(Vec3::new(0., 0., 0.)),

        Collider::cuboid(2000., 0.5, 2000.),
        CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        RigidBody::Static,
    ));

    commands.spawn((
        ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("maptest.glb"),
        )),
        Transform::from_xyz(-10., 10., 3.)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(30.))
        ,
    ));

    commands.spawn((
        ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("fiebigershof.glb"),
        )),
        Transform::from_xyz(20., -20., 0.)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(50.))
        ,
    ));
}

fn spawn_enemies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut id_counter: ResMut<IDCounter>,
    mut net_id_map: ResMut<NetIDMap>,
    mut entity_map: ResMut<EntityMap>,
) {
    let mut rng = rand::rng();

    for _ in 0..2000 {
        let velocity = LinearVelocity(random_velocity(3., 9.));
        let position = random_position(HALF_BOUNDARY);
        let material = MeshMaterial3d(materials.add(Color::srgb(
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
        )));

        let enemy_radius = rng.random_range(1.0..2.0);

        // spawn enemy
        let id = commands.spawn((
            Transform::from_translation(position.extend(enemy_radius + 10.)),
            Mesh3d(meshes.add(Sphere::new(enemy_radius))),
            material,

            RigidBody::Dynamic,
            Collider::sphere(enemy_radius),
            velocity,
            CollisionLayers::new([Layer::Ball], [Layer::Boundary]),
            Restitution::new(1.0), // Perfect bounce (1.0 = 100% energy retained)
            Friction::ZERO.with_combine_rule(CoefficientCombine::Min), // Remove friction

            Enemy,
            Radius(enemy_radius),
            LastBroadcast(HashMap::new()),
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
