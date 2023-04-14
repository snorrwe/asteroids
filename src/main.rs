#![windows_subsystem = "windows"]

mod collision;

use std::num::Wrapping;
use std::time::Duration;

use collision::{aabb_bundle, CollisionEvent, CollisionPlugin, CollisionTag, Collisions, AABB};
use engine::assets::Handle;
use engine::audio::Audio;
use engine::camera::Camera3d;
use engine::cecs::commands::EntityCommands;
use engine::glam::{self, Vec2, Vec3};
use engine::renderer::sprite_renderer::{self, SpriteInstance, SpriteSheet};
use engine::renderer::{self, GraphicsState};
use engine::transform::{self, transform_bundle, GlobalTransform, Transform};
use engine::{
    assets, App, DefaultPlugins, DeltaTime, KeyBoardInputs, Plugin, Stage, Timer, VirtualKeyCode,
};

use engine::cecs::prelude::*;

use engine::quat_ext::{PrimaryAxis, RotationExtension};

const MAP_RADIUS: f32 = 25.0;
const TARGET: usize = 100;
const MAX_ACC: f32 = 25.0;
const MAX_VEL: f32 = 12.0;
const INERTIA: f32 = 1.0;

const ASTEROID_TAG: CollisionTag = CollisionTag { src: 1, dst: 0xFE };
const BULLET_TAG: CollisionTag = CollisionTag {
    src: 1 << 1,
    dst: 1,
};
const PLAYER_TAG: CollisionTag = CollisionTag {
    src: 1 << 2,
    dst: 1,
};

struct Asteroid;
struct Bullet;
struct LifeTime(pub Timer);

struct Score {
    pub score: Wrapping<u64>,
}

#[derive(Default)]
struct Player {
    pub velocity: f32,
    pub acceleration: f32,
}

#[derive(Default)]
struct Velocity(pub Vec2);

struct UniformAnimation {
    pub timer: Timer,
    pub n: u32,
}
struct RotationTime(pub Duration);
struct PlayerCamera {
    follow_speed: f32,
}

struct AsteroidAssets {
    pub sheet: Handle<SpriteSheet>,
    pub n: u32,
}

struct BulletAssets {
    pub sheet: Handle<SpriteSheet>,
    pub n: u32,
}

struct Thrust;
struct ThrustAssets {
    pub sheet: Handle<SpriteSheet>,
    pub n: u32,
}

fn rotator(dt: Res<DeltaTime>, mut q: Query<&mut transform::Transform, With<Asteroid>>) {
    let dt = dt.0.as_secs_f32();
    q.par_for_each_mut(|tr| {
        tr.rot = tr.rot.rotate_around_self(PrimaryAxis::Z, dt);
    });
}

fn sprite_animator(dt: Res<DeltaTime>, mut q: Query<(&mut SpriteInstance, &mut UniformAnimation)>) {
    let dt = dt.0;
    q.par_for_each_mut(|(s, anim)| {
        anim.timer.update(dt);
        if anim.timer.just_finished() {
            s.index = (s.index + 1) % anim.n;
        }
    });
}

fn update_lifetime(mut cmd: Commands, mut q: Query<(EntityId, &mut LifeTime)>, dt: Res<DeltaTime>) {
    for (id, lt) in q.iter_mut() {
        lt.0.update(dt.0);
        if lt.0.just_finished() {
            cmd.delete(id);
        }
    }
}

fn split_asteroid(
    cmd: &mut Commands,
    v: &Velocity,
    tr: &Transform,
    assets: &AsteroidAssets,
    scale: f32,
) {
    let vel_mag = v.0.length() * 1.05;
    let v = v.0.normalize_or_zero();
    let mut tr = tr.clone();
    tr.scale *= scale;
    {
        let rot = fastrand::f32() * std::f32::consts::TAU;
        let (s, c) = rot.sin_cos();
        let v = Vec2::new(v.x * c + v.y * s, v.x * -s + v.y * c) * vel_mag;
        spawn_asteroid(
            cmd.spawn(),
            tr.clone(),
            assets.sheet.clone(),
            fastrand::u32(..assets.n),
            Velocity(v),
        );
    }
    {
        let rot = fastrand::f32() * std::f32::consts::TAU;
        let (s, c) = rot.sin_cos();
        let v = Vec2::new(v.x * c + v.y * s, v.x * -s + v.y * c) * vel_mag;
        spawn_asteroid(
            cmd.spawn(),
            tr,
            assets.sheet.clone(),
            fastrand::u32(..assets.n),
            Velocity(v),
        );
    }
}

fn handle_collisions(
    collisions: Res<Collisions>,
    mut cmd: Commands,
    q_asteroid: Query<(&Velocity, &GlobalTransform)>,
    mut score: ResMut<Score>,
    asteroid_assets: Res<AsteroidAssets>,
) {
    for event in collisions.0.iter() {
        let CollisionEvent {
            entity_1,
            tag1,
            entity_2,
            tag2,
        } = event;
        const SPLIT_SCALE: f32 = 0.8;
        // at most 3 splits
        const MIN_SCALE: f32 = SPLIT_SCALE * SPLIT_SCALE * SPLIT_SCALE;
        if tag1 == &ASTEROID_TAG && tag2 == &BULLET_TAG {
            score.score += 1;
            cmd.delete(*entity_1);
            cmd.delete(*entity_2);
            if let Some((v, tr)) = q_asteroid.fetch(*entity_1) {
                if tr.0.scale.x > MIN_SCALE {
                    split_asteroid(&mut cmd, v, &tr.0, &asteroid_assets, SPLIT_SCALE);
                }
            }
        } else if tag2 == &ASTEROID_TAG && tag1 == &BULLET_TAG {
            score.score += 1;
            cmd.delete(*entity_1);
            cmd.delete(*entity_2);
            if let Some((v, tr)) = q_asteroid.fetch(*entity_2) {
                if tr.0.scale.x > MIN_SCALE {
                    split_asteroid(&mut cmd, v, &tr.0, &asteroid_assets, SPLIT_SCALE);
                }
            }
        }
    }
}

fn camera_controller(
    dt: Res<DeltaTime>,
    q_player: Query<&GlobalTransform, With<Player>>,
    mut q_cam: Query<(&mut Transform, &PlayerCamera)>,
) {
    let Some(tr) = q_player.single() else {
        return;
    };
    let player_pos = tr.0.pos;

    for (tr, cam) in q_cam.iter_mut() {
        let d = player_pos - tr.pos;
        tr.pos += d * dt.0.as_secs_f32() * cam.follow_speed;
        const PADDING_X: f32 = 20.0;
        const PADDING_Y: f32 = 12.0;
        tr.pos.x = tr
            .pos
            .x
            .clamp(-MAP_RADIUS + PADDING_X, MAP_RADIUS - PADDING_X);
        tr.pos.y = tr
            .pos
            .y
            .clamp(-MAP_RADIUS + PADDING_Y, MAP_RADIUS - PADDING_Y);
    }
}

fn wraparound_system(mut q: Query<(&mut Transform, &GlobalTransform)>) {
    q.par_for_each_mut(|(tr, g)| {
        let g = &g.0;
        if g.pos.x < -MAP_RADIUS {
            tr.pos.x += 2.0 * MAP_RADIUS;
        }
        if g.pos.y < -MAP_RADIUS {
            tr.pos.y += 2.0 * MAP_RADIUS;
        }
        if MAP_RADIUS < g.pos.x {
            tr.pos.x -= 2.0 * MAP_RADIUS;
        }
        if MAP_RADIUS < g.pos.y {
            tr.pos.y -= 2.0 * MAP_RADIUS;
        }
    });
}

fn spawn_asteroid(
    cmd: &mut EntityCommands,
    transform: Transform,
    sheet: Handle<SpriteSheet>,
    index: u32,
    vel: Velocity,
) {
    cmd.insert_bundle(sprite_renderer::sprite_sheet_bundle(
        sheet,
        SpriteInstance {
            index,
            flip: fastrand::bool(),
        },
    ))
    .insert_bundle((Asteroid, vel))
    .insert_bundle(aabb_bundle(
        AABB::around_origin(Vec2::splat(0.8)),
        ASTEROID_TAG,
    ))
    .insert_bundle(transform_bundle(transform));
}

fn spawn_asteroids_system(
    q_asteroid: Query<&(), With<Asteroid>>,
    mut cmd: Commands,
    assets: Res<AsteroidAssets>,
    q_player: Query<&GlobalTransform, With<Player>>,
) {
    let count = q_asteroid.count();

    let Some(player_pos) = q_player.single() else {
        return;
    };

    for _ in (count..TARGET).take(5) {
        let mut pos = Vec3::ZERO;
        loop {
            pos.x = fastrand::f32() * 2.0 * MAP_RADIUS - MAP_RADIUS;
            pos.y = fastrand::f32() * 2.0 * MAP_RADIUS - MAP_RADIUS;
            if pos.distance(player_pos.0.pos) > 5.0 {
                break;
            }
        }
        let rot = glam::Quat::from_axis_angle(Vec3::Z, fastrand::f32() * std::f32::consts::TAU);

        let vx = fastrand::f32();
        let vy = fastrand::f32();
        let vrot = fastrand::f32();
        let (vc, vs) = vrot.sin_cos();

        let vel = Vec2::new(vx * vc - vy * vs, vx * vc + vy * vs);

        spawn_asteroid(
            cmd.spawn(),
            transform::Transform {
                pos,
                rot,
                scale: Vec3::ONE,
            },
            assets.sheet.clone(),
            fastrand::u32(0..assets.n),
            Velocity(vel),
        );
    }
}

fn setup_bullets(
    mut cmd: Commands,
    graphics_state: Res<GraphicsState>,
    mut assets: ResMut<assets::Assets<sprite_renderer::SpriteSheet>>,
) {
    // init sprite
    let bytes = include_bytes!("../assets/bullet.png");
    let graphics_state: &GraphicsState = &graphics_state;
    let assets: &mut assets::Assets<sprite_renderer::SpriteSheet> = &mut assets;
    let texture = renderer::texture::Texture::from_bytes(
        graphics_state.device(),
        graphics_state.queue(),
        bytes,
        "bullet",
    )
    .unwrap();
    let n = 2;
    let sprite_sheet =
        sprite_renderer::SpriteSheet::from_texture(Vec2::ZERO, Vec2::splat(128.0), n, texture);

    let handle = assets.insert(sprite_sheet);

    cmd.insert_resource(BulletAssets {
        sheet: handle.clone(),
        n,
    });
}

fn player_rotation_system(
    dt: Res<DeltaTime>,
    inputs: Res<KeyBoardInputs>,
    mut q: Query<(&mut transform::Transform, &mut RotationTime)>,
) {
    for (tr, rot_time) in q.iter_mut() {
        let mut rot = 0.0;
        for k in inputs.pressed.iter() {
            match k {
                VirtualKeyCode::D => rot += 1.0,
                VirtualKeyCode::A => rot -= 1.0,
                _ => continue,
            }
        }

        if rot != 0.0 {
            // ramp up rotation speed
            rot_time.0 = (rot_time.0 + dt.0).max(Duration::from_millis(300));
            let rotation_velocity = rot_time.0.as_secs_f32() * 3.0;
            let rotation = rot * dt.0.as_secs_f32() * rotation_velocity;
            tr.rot = tr.rot.rotate_around_self(PrimaryAxis::Z, rotation);
        } else {
            rot_time.0 = Default::default();
        }
    }
}

fn player_thrust_system(
    dt: Res<DeltaTime>,
    inputs: Res<KeyBoardInputs>,
    mut q: Query<(EntityId, &transform::Transform, &mut Velocity, &mut Player)>,
    mut cmd: Commands,
    thruster: Res<ThrustAssets>,
    thrusters: Query<EntityId, With<Thrust>>,
) {
    let dt = dt.0.as_secs_f32();
    if let Some((id, tr, vel, player)) = q.single_mut() {
        for key in inputs.just_released.iter() {
            if let VirtualKeyCode::W = key {
                player.acceleration = 0.0;
                player.velocity = vel.0.length();
                for id in thrusters.iter() {
                    cmd.delete(id);
                }
            }
        }
        for key in inputs.just_pressed.iter() {
            if let VirtualKeyCode::W = key {
                transform::spawn_child(id, &mut cmd, |cmd| {
                    cmd.insert_bundle(transform_bundle(Transform::from_position(Vec3::new(
                        0.0, -0.5, 0.1,
                    ))))
                    .insert_bundle((
                        Thrust,
                        UniformAnimation {
                            timer: Timer::new(Duration::from_millis(100), true),
                            n: thruster.n,
                        },
                    ))
                    .insert_bundle(sprite_renderer::sprite_sheet_bundle(
                        thruster.sheet.clone(),
                        None,
                    ));
                });
            }
        }
        if !inputs.pressed.iter().any(|k| match k {
            VirtualKeyCode::W => {
                // max acceleration in 0.3 seconds
                player.acceleration = (player.acceleration + dt * MAX_ACC * 3.0).min(MAX_ACC);
                player.velocity = (player.velocity + player.acceleration * dt).min(MAX_VEL);
                vel.0 = vel
                    .0
                    .lerp((tr.rot * Vec3::Y).truncate() * player.velocity, dt);
                true
            }
            _ => false,
        }) {
            player.velocity = (vel.0.length() - dt * INERTIA).max(0.0);
            vel.0 = vel.0.normalize_or_zero() * player.velocity;
        }
    }
}

fn move_system(dt: Res<DeltaTime>, mut q: Query<(&mut Transform, &Velocity)>) {
    let dt = dt.0.as_secs_f32();
    q.par_for_each_mut(|(tr, v)| {
        tr.pos += v.0.extend(0.0) * dt;
    });
}

struct FireSound;
fn setup_slash(mut cmd: Commands, mut assets: ResMut<assets::Assets<Audio>>) {
    let bytes = include_bytes!("../assets/slash.mp3");
    let music = Audio::load_audio_bytes(bytes, &mut assets).unwrap();
    cmd.spawn().insert_bundle((FireSound, music));
}

fn fire_system(
    slash: Query<&assets::Handle<Audio>, With<FireSound>>,
    inputs: Res<KeyBoardInputs>,
    audio: Res<assets::Assets<Audio>>,
    am: Res<engine::audio::AudioManager>,
    bullet_assets: Res<BulletAssets>,
    mut cmd: Commands,
    q_player: Query<(&GlobalTransform, &Player)>,
) {
    for key in inputs.just_released.iter() {
        if matches!(key, VirtualKeyCode::Space) {
            if let Some(s) = slash.single() {
                let music = audio.get(s);
                am.play(music);
            }
            if let Some((tr, player)) = q_player.single() {
                let rot = tr.0.rot;
                let v = tr.0.rot * Vec3::Y;
                let vel = v * (1.0 + player.velocity).min(MAX_VEL + 1.0);
                let pos = tr.0.pos + v * 0.5;

                cmd.spawn()
                    .insert_bundle(sprite_renderer::sprite_sheet_bundle(
                        bullet_assets.sheet.clone(),
                        None,
                    ))
                    .insert_bundle((
                        LifeTime(Timer::new(Duration::from_secs(5), false)),
                        Bullet,
                        UniformAnimation {
                            timer: Timer::new(Duration::from_millis(100), true),
                            n: bullet_assets.n,
                        },
                        Velocity(vel.truncate()),
                    ))
                    .insert_bundle(aabb_bundle(
                        AABB::around_origin(Vec2::new(0.25, 0.5)),
                        BULLET_TAG,
                    ))
                    .insert_bundle(transform::transform_bundle(transform::Transform {
                        pos,
                        rot,
                        scale: Vec3::splat(0.2),
                    }));
            }
        }
    }
}

fn setup_asteroids(
    mut cmd: Commands,
    graphics_state: Res<GraphicsState>,
    mut assets: ResMut<assets::Assets<sprite_renderer::SpriteSheet>>,
) {
    // init sprite
    let bytes = include_bytes!("../assets/asteroids.png");
    let n = 2;
    let graphics_state: &GraphicsState = &graphics_state;
    let assets: &mut assets::Assets<sprite_renderer::SpriteSheet> = &mut assets;
    let texture = renderer::texture::Texture::from_bytes(
        graphics_state.device(),
        graphics_state.queue(),
        bytes,
        "asteroids",
    )
    .unwrap();
    let sprite_sheet =
        sprite_renderer::SpriteSheet::from_texture(Vec2::ZERO, Vec2::splat(128.0), n, texture);

    let handle = assets.insert(sprite_sheet);

    cmd.insert_resource(AsteroidAssets {
        sheet: handle.clone(),
        n,
    });
}

fn setup_player(
    mut cmd: Commands,
    graphics_state: Res<GraphicsState>,
    mut assets: ResMut<assets::Assets<sprite_renderer::SpriteSheet>>,
) {
    let bytes = include_bytes!("../assets/ship.png");
    let box_size = Vec2::new(32.0, 45.0);
    let sprite_handle = {
        let graphics_state: &GraphicsState = &graphics_state;
        let assets: &mut assets::Assets<sprite_renderer::SpriteSheet> = &mut assets;

        // setup thruster
        {
            let bytes = include_bytes!("../assets/flame.png");
            let box_size = Vec2::new(32.0, 32.0);
            let texture = renderer::texture::Texture::from_bytes(
                graphics_state.device(),
                graphics_state.queue(),
                bytes,
                "thrust",
            )
            .unwrap();
            let n = 4;
            let sprite_sheet =
                sprite_renderer::SpriteSheet::from_texture(Vec2::ZERO, box_size, n, texture);

            let handle = assets.insert(sprite_sheet);
            cmd.insert_resource(ThrustAssets { sheet: handle, n });
        }

        let texture = renderer::texture::Texture::from_bytes(
            graphics_state.device(),
            graphics_state.queue(),
            bytes,
            "ship",
        )
        .unwrap();
        let sprite_sheet =
            sprite_renderer::SpriteSheet::from_texture(Vec2::ZERO, box_size, 1, texture);

        assets.insert(sprite_sheet)
    };
    // player
    cmd.spawn()
        .insert_bundle(transform::transform_bundle(
            transform::Transform::from_scale(Vec3::splat(0.5)),
        ))
        .insert_bundle(sprite_renderer::sprite_sheet_bundle(
            sprite_handle.clone(),
            None,
        ))
        .insert_bundle(aabb_bundle(
            AABB::around_origin(Vec2::splat(0.5)),
            PLAYER_TAG,
        ))
        .insert_bundle((
            Player::default(),
            Velocity::default(),
            RotationTime(Duration::default()),
        ));

    // camera
    cmd.spawn()
        .insert(PlayerCamera { follow_speed: 5.0 })
        .insert_bundle(renderer::camera_bundle(Camera3d {
            eye: Vec3::new(0.0, 0.0, 20.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            aspect: 16.0 / 9.0,
            fovy: 45.0,
            znear: 5.0,
            zfar: 50.0,
        }))
        .insert_bundle(transform_bundle(transform::Transform::default()));
}

struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(self, app: &mut App) {
        app.stage(Stage::Update)
            .add_system(rotator)
            .add_system(sprite_animator)
            .add_system(player_rotation_system)
            .add_system(player_thrust_system)
            .add_system(camera_controller.after(player_thrust_system))
            .add_system(fire_system)
            .add_system(spawn_asteroids_system)
            .add_system(wraparound_system)
            .add_system(update_lifetime)
            .add_system(move_system);

        app.stage(Stage::PostUpdate).add_system(handle_collisions);

        app.add_startup_system(setup_asteroids)
            .add_startup_system(setup_player)
            .add_startup_system(setup_bullets)
            .add_startup_system(setup_slash);

        app.insert_resource(Score { score: Wrapping(0) });
    }
}

fn main() {
    let mut app = App::default();
    app.add_plugin(DefaultPlugins);
    app.add_plugin(GamePlugin);
    app.add_plugin(CollisionPlugin);
    pollster::block_on(app.run());
}
