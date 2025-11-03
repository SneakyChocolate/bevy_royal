use bevy_royal::*;

fn main() {
    App::new()
        .insert_resource(CursorPos(Vec2::ZERO))
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(Startup, spawn_enemies)
        .add_systems(Update, (cursor_position_system, player_movement_system, apply_velocity_system, enemy_kill_system))
        .run();
}


fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Tonemapping::TonyMcMapface, // 1. Using a tonemapper that desaturates to white is recommended
        Bloom::default(),           // 2. Enable bloom for the camera
        DebandDither::Enabled,      // Optional: bloom causes gradients which cause banding
        Transform::from_xyz(200., 0., 1.),
        Player,
        Alive(true),
        Radius(20.),
        Velocity(Vec2::new(0., 0.)),
        Mesh2d(meshes.add(Circle::new(20.))),
        // 3. Put something bright in a dark environment to see the effect
        MeshMaterial2d(materials.add(Color::srgb(0., 1., 0.))),
    ));
}

fn spawn_enemies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let mut rng = rand::rng();
    for _ in 0..100 {
        let velocity = Velocity(random_velocity());
        let position = random_position(1000.);
        let material = MeshMaterial2d(materials.add(Color::srgb(
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
            rng.random_range(0.0..4.0),
        )));

        // Circle mesh
        commands.spawn((
            Mesh2d(meshes.add(Circle::new(40.))),
            // 3. Put something bright in a dark environment to see the effect
            material,
            Transform::from_translation(position.extend(0.)),
            velocity,
            Enemy,
            Radius(40.),
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
    query: Query<(&mut Velocity, &Alive), With<Player>>,
) {
    for (mut velocity, alive) in query {
        if alive.0 {
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
    }
}

fn apply_velocity_system(
    time: Res<Time>,
    query: Query<(&mut Transform, &Velocity)>,
) {
    let d = time.delta_secs();
    for (mut transform, velocity) in query {
        transform.translation += velocity.0.extend(0.) * d;
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
