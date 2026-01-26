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
    pub fn send_to(&self, bytes: &[u8], addr: SocketAddr, byte_count: &mut usize) -> bool {
        match self.socket.send_to(bytes, addr) {
            Ok(l) => {
                let r = l == bytes.len();
                if r {*byte_count += l;}
                r
            },
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
        let mut delay_pool: Vec<(f32, ( SocketAddr, ClientMessage ))> = Vec::with_capacity(1000);
        let mut past = std::time::Instant::now();
        let mut last_sent_bytes = std::time::Instant::now();
        let mut last_received_bytes = std::time::Instant::now();

        let mut reliable_counter = 1;
        let mut reliable_packages = HashMap::<usize, ReliablePackage>::new();

        let mut sent_byte_count: usize = 0;
        let mut received_byte_count: usize = 0;

        loop {
            sent_byte_count = 0;
            received_byte_count = 0;

            // delta time
            let present = std::time::Instant::now();

            let delta_secs = present.duration_since(past).as_secs_f32();
            past = present;

            let delta_secs_last_sent_bytes = present.duration_since(last_sent_bytes).as_secs_f32();
            let delta_secs_last_received_bytes = present.duration_since(last_received_bytes).as_secs_f32();

            // resend all important messegaes if they werent confirmed yet
            let now = std::time::Instant::now();
            for (_, packet) in reliable_packages.iter_mut() {
                if now.duration_since(packet.last_send) > std::time::Duration::from_millis(300) {
                    server_socket.send_to(&packet.bytes, packet.addr, &mut sent_byte_count);
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
                server_socket.send_to(&bytes, addr, &mut sent_byte_count);
            }

            // get from socket
            let ServerSocket { socket, buf } = &mut server_socket;

            while let Ok((len, addr)) = socket.recv_from(buf) {
                if let Some(ClientMessage {reliable, message: client_message}) = ClientMessage::decode(&buf[..len]) {
                    received_byte_count += len;
                    if let ClientMessageInner::Confirm(reliable) = &client_message {
                        reliable_packages.remove(reliable);
                    }
                    delay_pool.push((0.0, (addr, ClientMessage {reliable, message: client_message})));
                }
            }

            // go through delay pool
            let mut removed = Vec::<( SocketAddr, ClientMessage )>::new();
            delay_pool.retain_mut(|(d, sm)| {
                *d += delta_secs;
                if *d >= 0.1 {
                    removed.push(sm.clone());
                    false
                }
                else {
                    true
                }
            });

            for message in removed {
                incoming_sender.send(message).unwrap();
            }

            // print bytes per second
            if sent_byte_count > 0 && delta_secs_last_sent_bytes != 0. {
                // info!("upload per second: {}", sent_byte_count as f32 / delta_secs_last_sent_bytes / 1000000.);
                last_sent_bytes = present;
            }
            if received_byte_count > 0 && delta_secs_last_received_bytes != 0. {
                // info!("download per second: {}", received_byte_count as f32 / delta_secs_last_received_bytes / 1000000.);
                last_received_bytes = present;
            }

            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    App::new()
        .insert_resource(IncomingReceiver(incoming_receiver))
        .insert_resource(OutgoingSender(outgoing_sender))
        .insert_resource(Gravity(Vec3::NEG_Z * 19.))
        // .insert_resource(Gravity::ZERO)
        .insert_resource(IDCounter(0))
        .insert_resource(EntityMap::default())
        .insert_resource(NetIDMap::default())
        .insert_resource(ClientPlayerMap::default())
        .add_plugins(DefaultPlugins)
        .add_plugins(PhysicsPlugins::default())
        .add_plugins(UnixTimePlugin)
        .add_systems(Startup, (
            setup,
            spawn_enemies,
            spawn_walls,
        ))
        .add_systems(Update, (
            receive_messages,
            enemy_kill_system,
            server_process_hits,
            broadcast_enemy_spawns,
            broadcast_player_spawns,
            (
                update_per_distance_setter_increase,
                (
                    broadcast_positions,
                    broadcast_velocities,
                    broadcast_player_looks,
                ),
                update_per_distance_setter_reset,
            ).chain(),
            broadcast_health,
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

#[derive(Resource, Default)]
struct ClientPlayerMap(HashMap<SocketAddr, Entity>);

#[derive(Resource)]
struct IDCounter(pub NetIDType);

/// attached to entities that are being sent to players / clients
#[derive(Component)]
pub struct LastBroadcast(pub HashMap<SocketAddr, f32>);

#[derive(Component)]
struct PendingSpawn;

#[derive(Component, Default)]
struct PlayerLook(MyQuat);

#[derive(Component)]
pub struct UpdateAddress {
    addr: SocketAddr,
}

#[derive(Component)]
struct Shooter {
    owner: Entity,
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
    mut player_look_query: Query<&mut PlayerLook>,
    client_addresses: Query<Entity, With<UpdateAddress>>,
    mut client_player_map: ResMut<ClientPlayerMap>,
) {
    while let Ok((addr, ClientMessage {reliable, message: client_message})) = incoming_receiver.0.try_recv() {

        if reliable > 0 {
            outgoing_sender.0.send((addr, ServerMessage::confirm(reliable)));
        }
        match client_message {

            ClientMessageInner::Confirm(_) => {},

            ClientMessageInner::Login => {
                if let Some(entity) = client_player_map.0.get(&addr) {
                    println!("duplicated login denied");
                }
                else {
                    println!("login");
                    // spawn player
                    let player_radius = 1.5;
                    let id = commands.spawn((
                        Transform::from_xyz(0., 0., player_radius + 10.)
                            .with_rotation(Quat::from_rotation_x(90_f32.to_radians())),
                        Player,
                        Health(100.),
                        Radius(player_radius),
                        PlayerLook::default(),
                        Mesh3d(meshes.add(Sphere::new(player_radius))),
                        MeshMaterial3d(materials.add(Color::srgb(0., 1., 0.))),
                        UpdateAddress {addr},
                        PendingSpawn,
                        LastBroadcast(HashMap::new()),
                    )).insert((
                        LinearVelocity(Vec3::new(10., -10., 0.)),
                        RigidBody::Dynamic,
                        CollisionLayers::new([Layer::Player], [Layer::Boundary]),
                        Collider::capsule(0.4, player_radius),
                        LockedAxes::ROTATION_LOCKED,
                        SweptCcd::default(),
                    )).id();

                    client_player_map.0.insert(addr, id);
                    net_id_map.0.insert(id, id_counter.0);
                    entity_map.0.insert(id_counter.0, id);
                    outgoing_sender.0.send((addr, ServerMessage::ok(1, id_counter.0))).unwrap();

                    id_counter.0 += 1;

                    // give all clients pending spawn
                    for client in client_addresses {
                        commands.entity(client).insert(PendingSpawn);
                    }
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

            ClientMessageInner::Jump(player_net_id) => {
                let player_entity_option = entity_map.0.get(&player_net_id);
                let mut player_exists = false;
                match player_entity_option {
                    Some(player_entity) => {
                        if let Ok((mut player_velocity, _)) = player_query.get_mut(*player_entity) {
                            player_exists = true;
                            player_velocity.0.z = 10.;
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
                        if let Ok(mut player_look) = player_look_query.get_mut(*player_entity) {
                            player_exists = true;
                            player_look.0 = rotation;
                        }
                    },
                    None => {},
                }
                if !player_exists {
                    entity_map.0.remove(&player_net_id);
                }
            },

            ClientMessageInner::Shoot(player_net_id, direction) => {
                let player_entity_option = entity_map.0.get(&player_net_id);
                let mut player_exists = false;
                match player_entity_option {
                    Some(player_entity) => {
                        if let Ok(( velocity, transform )) = player_query.get(*player_entity) {
                            player_exists = true;
                            info!("client shot");

                            commands.spawn((
                                RayCaster::new(transform.translation, Dir3::new_unchecked(direction.into())),
                                Shooter { owner: *player_entity },
                            ));
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
    materials: ResMut<Assets<StandardMaterial>>,
    net_id_map: ResMut<NetIDMap>,
    client_addresses: Query<(Entity, &UpdateAddress), With<PendingSpawn>>,
    player_query: Query<(Entity, &Transform, &PlayerVelocityType, &MeshMaterial3d<StandardMaterial>, &Player, &Health, &Radius)>,
) {
    for (id, addr) in client_addresses.iter() {
        // println!("client spawn");
        let mut entity_packages = Vec::<EntityPackage>::new();
        for (entity, transform, velocity, meshmaterial3d, player, health, radius) in &player_query {
            println!("player broadcast");
            let net_id = net_id_map.0.get(&entity).unwrap();
            entity_packages.push(EntityPackage { net_id: *net_id, components: vec![
                NetComponent::Capsule(0.4, radius.0),
                (*transform).into(),
                (*velocity).into(),
                (materials.get(meshmaterial3d).unwrap().clone()).into(),
                (*player).into(),
                (*health).into(),
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
const PLAYER_LOOK_PACKAGES_PER_MESSAGE: usize = (1000. / std::mem::size_of::<PlayerLookPackage>() as f32).floor() as usize;
const HEALTH_PACKAGES_PER_MESSAGE: usize = (1000. / std::mem::size_of::<HealthPackage>() as f32).floor() as usize;

fn update_per_distance_check(lb: f32, distance: f32) -> bool {
   lb >= distance / 500. + 0.01
}

fn update_per_distance_reset(
    addr: SocketAddr,
    delta_secs: f32,
    last_broadcast_option: Option<Mut<LastBroadcast>>,
    distance: f32,
) -> bool {
    if let Some(mut last_broadcast) = last_broadcast_option {
        let mut lb = last_broadcast.0.entry(addr).or_insert(0.);
        let updating = update_per_distance_check(*lb, distance);
        if updating {*lb = 0.0;}
        updating
    }
    else {
        false
    }
}
fn update_per_distance_increase(
    addr: SocketAddr,
    delta_secs: f32,
    last_broadcast_option: Option<Mut<LastBroadcast>>,
    distance: f32,
) -> bool {
    if let Some(mut last_broadcast) = last_broadcast_option {
        let mut lb = last_broadcast.0.entry(addr).or_insert(0.);
        *lb += delta_secs;
        let updating = update_per_distance_check(*lb, distance);
        updating
    }
    else {
        false
    }
}
fn update_per_distance(
    addr: SocketAddr,
    delta_secs: f32,
    last_broadcast_option: Option<&LastBroadcast>,
    distance: f32,
) -> bool {
    if let Some(mut last_broadcast) = last_broadcast_option {
        let lb = last_broadcast.0.get(&addr).cloned().unwrap_or_default();
        update_per_distance_check(lb, distance)
    }
    else {
        false
    }
}

fn update_per_distance_setter_increase(
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, &mut LastBroadcast)>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (_entity, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;

        // Collect enemies within radius for this specific player
        for (entity, entity_transform, last_broadcast) in &mut query {
            let distance = player_pos.distance(entity_transform.translation);
            update_per_distance_increase(addr.addr, delta_secs, Some( last_broadcast ), distance);
        }
    }
}

fn update_per_distance_setter_reset(
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, &mut LastBroadcast)>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (_entity, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;

        // Collect enemies within radius for this specific player
        for (entity, entity_transform, last_broadcast) in &mut query {
            let distance = player_pos.distance(entity_transform.translation);
            update_per_distance_reset(addr.addr, delta_secs, Some( last_broadcast ), distance);
        }
    }
}

fn broadcast_health(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Health), Changed<Health>>,
    net_id_map: ResMut<NetIDMap>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (_entity, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;

        // Collect enemies within radius for this specific player
        let nearby_entities: Vec<HealthPackage> = query
            .iter_mut()
            .filter_map(|(entity, entity_health)| {
                let net_id = net_id_map.0.get(&entity)?;

                Some(HealthPackage {
                    net_id: *net_id,
                    health: entity_health.0,
                })
            })
            .collect();

        // Split into chunks and send
        for chunk in nearby_entities.chunks(HEALTH_PACKAGES_PER_MESSAGE) {
            let message = ServerMessage::update_healths(chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message)).unwrap();
        }
    }
}

fn broadcast_player_looks(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, &PlayerLook, Option<&LastBroadcast>)>,
    net_id_map: ResMut<NetIDMap>,
    time: Res<Time>,
) {
    let delta_secs = time.delta_secs();

    // Process each client separately
    for (_entity, addr, player_transform) in client_addresses.iter() {
        let player_pos = player_transform.translation;

        // Collect enemies within radius for this specific player
        let nearby_entities: Vec<PlayerLookPackage> = query
            .iter_mut()
            .filter_map(|(entity, entity_transform, player_look, last_broadcast_option)| {
                let distance = player_pos.distance(entity_transform.translation);
                let net_id = net_id_map.0.get(&entity)?;

                if update_per_distance(addr.addr, delta_secs, last_broadcast_option, distance) {
                    Some(PlayerLookPackage {
                        net_id: *net_id,
                        rotation: player_look.0,
                    })
                }
                else {
                    None
                }
            })
            .collect();

        // Split into chunks and send
        for chunk in nearby_entities.chunks(PLAYER_LOOK_PACKAGES_PER_MESSAGE) {
            let message = ServerMessage::update_player_looks(chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message)).unwrap();
        }
    }
}

fn broadcast_positions(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, Option<&LastBroadcast>)>,
    net_id_map: ResMut<NetIDMap>,
    time: Res<Time>,
    unix_time: Res<UnixTime>,
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
            let message = ServerMessage::update_positions(unix_time.0, chunk.to_vec());
            outgoing_sender.0.send((addr.addr, message)).unwrap();
        }
    }
}

fn broadcast_velocities(
    outgoing_sender: Res<OutgoingSender>,
    client_addresses: Query<(Entity, &UpdateAddress, &Transform)>,
    mut query: Query<(Entity, &Transform, &LinearVelocity, Option<&LastBroadcast>)>,
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
        Transform::from_xyz(0., 0., 1500.).looking_at(Vec3::ZERO, Vec3::Y),
        Tonemapping::TonyMcMapface,
        Bloom::default(),
        DebandDither::Enabled,
    ));

    // commands.spawn((
    //     Mesh3d(meshes.add(Plane3d::default().mesh().size(2000.0, 2000.0).subdivisions(10))),
    //     MeshMaterial3d(standard_materials.add(Color::srgb(0.4, 0.5, 0.1))),
    //     Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
    //         .with_translation(Vec3::new(0., 0., 0.)),

    //     Collider::cuboid(2000., 0.5, 2000.),
    //     CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
    //     RigidBody::Static,
    // ));

    // sun
    commands.spawn((
        DirectionalLight {
            illuminance: 2000.0,
            ..default()
        },
        Transform::from_xyz(0.0, 2.0, 0.0).with_rotation(Quat::from_rotation_x(-std::f32::consts::PI / 4.)),
    ));

    commands.spawn((
        ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        RigidBody::Static,
        CollisionMargin(0.5),

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("map_shooter12.glb"),
        )),
        map_transform(),
    ));

    commands.spawn((
        ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        CollisionLayers::new([Layer::Boundary], [Layer::Ball, Layer::Player]),
        RigidBody::Static,

        SceneRoot(asset_server.load(
            GltfAssetLabel::Scene(0).from_asset("house1.glb"),
        )),
        map_transform(),
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
            .with_scale(Vec3::splat(15.))
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

    for _ in 0..100 {
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

fn enemy_kill_system(
    players: Query<(&mut Health, &Transform, &Radius), With<Player>>,
    enemies: Query<(&Transform, &Radius), With<Enemy>>,
) {
    for (mut player_health, player_pos, player_radius) in players {
        for (enemy_pos, enemy_radius) in enemies {
            let distance = player_pos.translation.distance(enemy_pos.translation);
            if distance - player_radius.0 - enemy_radius.0 <= 0. {
                player_health.0 = 0.;
            }
        }
    }
}

fn server_process_hits(
    mut commands: Commands,
    query: Query<(Entity, &RayCaster, &RayHits, &Shooter)>,
    mut health_q: Query<&mut Health>,
    mut velocity_q: Query<&mut Velocity>,
) {
    for (ray_entity, ray, hits, shooter) in &query {
        // Find first hit that is NOT the shooter
        let valid_hit = hits
            .iter_sorted()
            .find(|hit| hit.entity != shooter.owner);

        if let Some(hit) = valid_hit {
            let hit_entity = hit.entity;

            // Damage
            if let Ok(mut health) = health_q.get_mut(hit_entity) {
                health.0 -= 10.;
                if health.0 < 0. {
                    health.0 = 0.;
                }
            }

            info!(
                "Shooter {:?} hit {:?} at {}",
                shooter.owner, hit_entity, hit.distance
            );
        }

        // One-frame ray
        commands.entity(ray_entity).despawn();
    }
}
