#![windows_subsystem = "windows"]

mod collision;

use std::num::Wrapping;
use std::time::Duration;
use std::usize;

use collision::{aabb_bundle, CollisionEvent, CollisionPlugin, CollisionTag, Collisions, AABB};
use engine::assets::{Assets, Handle};
use engine::camera::Camera3d;
use engine::cecs::commands::EntityCommands;
use engine::glam::{self, Vec2, Vec3};
use engine::renderer::sprite_renderer::{self, sprite_sheet_bundle, SpriteInstance, SpriteSheet};
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

/// Every entity that's part of the game logic (that needs to be deleted on restart)
struct GameEntity;
struct Asteroid;
struct Bullet;
struct LifeTime(pub Timer);

struct Score {
    pub score: Wrapping<u64>,
    pub rendered_score: u64,
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

struct Thrust;

#[derive(Default)]
struct Sprites {
    pub thrust_sheet: Handle<SpriteSheet>,
    pub thrust_n: u32,
    pub bullet_sheet: Handle<SpriteSheet>,
    pub bullet_n: u32,
    pub asteroid_sheet: Handle<SpriteSheet>,
    pub asteroid_n: u32,
    pub game_over_sheet: Handle<SpriteSheet>,
    pub player: Handle<SpriteSheet>,
    pub digits: Handle<SpriteSheet>,
}

struct ScoreDigit;

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

fn split_asteroid(cmd: &mut Commands, v: &Velocity, tr: &Transform, assets: &Sprites, scale: f32) {
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
            assets.asteroid_sheet.clone(),
            fastrand::u32(..assets.asteroid_n),
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
            assets.asteroid_sheet.clone(),
            fastrand::u32(..assets.asteroid_n),
            Velocity(v),
        );
    }
}

struct GameOver;

fn game_over(sprites: &Sprites, cmd: &mut EntityCommands, mut pos: Vec3) {
    // TODO: score
    pos.z = -1.0;
    let tr = Transform {
        pos,
        scale: Vec3::new(40.0, 20., 0.),
        ..Default::default()
    };
    cmd.insert_bundle(transform_bundle(tr))
        .insert_bundle(sprite_sheet_bundle(
            sprites.game_over_sheet.clone(),
            SpriteInstance {
                index: 0,
                flip: true,
            },
        ))
        .insert_bundle((
            GameOver,
            GameEntity,
            Cooldown(Timer::new(Duration::from_millis(500), false)),
        ));
}

fn handle_collisions(
    collisions: Res<Collisions>,
    mut cmd: Commands,
    q_asteroid: Query<(&Velocity, &GlobalTransform)>,
    q_camera_pos: Query<&GlobalTransform, With<Camera3d>>,
    mut score: ResMut<Score>,
    sprites: Res<Sprites>,
) {
    for event in collisions.0.iter() {
        let CollisionEvent {
            mut entity_1,
            mut tag1,
            mut entity_2,
            mut tag2,
        } = *event;
        const SPLIT_SCALE: f32 = 0.8;
        // at most 3 splits
        const MIN_SCALE: f32 = SPLIT_SCALE * SPLIT_SCALE * SPLIT_SCALE;
        if tag1 == ASTEROID_TAG && tag2 == BULLET_TAG {
            std::mem::swap(&mut entity_1, &mut entity_2);
            std::mem::swap(&mut tag1, &mut tag2);
        }
        if tag2 == ASTEROID_TAG && tag1 == BULLET_TAG {
            score.score += 1;
            cmd.delete(entity_1);
            cmd.delete(entity_2);
            if let Some((v, tr)) = q_asteroid.fetch(entity_2) {
                if tr.0.scale.x > MIN_SCALE {
                    split_asteroid(&mut cmd, v, &tr.0, &sprites, SPLIT_SCALE);
                }
            }
        }
        if tag2 == ASTEROID_TAG && tag1 == PLAYER_TAG {
            std::mem::swap(&mut entity_1, &mut entity_2);
            std::mem::swap(&mut tag1, &mut tag2);
        }
        if tag1 == ASTEROID_TAG && tag2 == PLAYER_TAG {
            cmd.entity(entity_2)
                .remove::<Player>()
                .remove::<CollisionTag>()
                .remove::<Velocity>();
            let pos = q_camera_pos.single().map(|tr| tr.0.pos).unwrap_or_default();
            game_over(&sprites, cmd.spawn(), pos);
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
    .insert_bundle((Asteroid, vel, GameEntity))
    .insert_bundle(aabb_bundle(
        AABB::around_origin(Vec2::splat(0.8)),
        ASTEROID_TAG,
    ))
    .insert_bundle(transform_bundle(transform));
}

fn spawn_asteroids_system(
    q_asteroid: Query<&(), With<Asteroid>>,
    mut cmd: Commands,
    assets: Res<Sprites>,
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
            assets.asteroid_sheet.clone(),
            fastrand::u32(0..assets.asteroid_n),
            Velocity(vel),
        );
    }
}

fn player_rotation_system(
    dt: Res<DeltaTime>,
    inputs: Res<KeyBoardInputs>,
    mut q: Query<(&mut transform::Transform, &mut RotationTime), With<Player>>,
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
    sprites: Res<Sprites>,
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
                        GameEntity,
                        UniformAnimation {
                            timer: Timer::new(Duration::from_millis(100), true),
                            n: sprites.thrust_n,
                        },
                    ))
                    .insert_bundle(sprite_renderer::sprite_sheet_bundle(
                        sprites.thrust_sheet.clone(),
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

struct Cooldown(pub Timer);

fn cooldown_system(
    mut cmd: Commands,
    dt: Res<DeltaTime>,
    mut cd: Query<(EntityId, &mut Cooldown)>,
) {
    for (id, cd) in cd.iter_mut() {
        cd.0.update(dt.0);
        if cd.0.just_finished() {
            cmd.entity(id).remove::<Cooldown>();
        }
    }
}

#[allow(unused)]
struct FireSound;

#[cfg(not(target_family = "wasm"))]
fn setup_slash(mut cmd: Commands, mut assets: ResMut<assets::Assets<engine::audio::Audio>>) {
    let bytes = include_bytes!("../assets/slash.mp3");
    let music = engine::audio::Audio::load_audio_bytes(bytes, &mut assets).unwrap();
    cmd.spawn().insert_bundle((FireSound, music));
}

#[cfg(target_family = "wasm")]
fn setup_slash() {}

fn fire_system(
    inputs: Res<KeyBoardInputs>,
    sprites: Res<Sprites>,
    mut cmd: Commands,
    q_player: Query<(&GlobalTransform, &Player)>,
    q_cd: Query<&(), (With<Cooldown>, With<Bullet>)>,

    #[cfg(not(target_family = "wasm"))] audio: Res<assets::Assets<engine::audio::Audio>>,
    #[cfg(not(target_family = "wasm"))] am: Res<engine::audio::AudioManager>,
    #[cfg(not(target_family = "wasm"))] slash: Query<
        &assets::Handle<engine::audio::Audio>,
        With<FireSound>,
    >,
) {
    if q_cd.single().is_some() {
        return;
    }

    for key in inputs.pressed.iter() {
        if let VirtualKeyCode::Space = key {
            if let Some((tr, player)) = q_player.single() {
                #[cfg(not(target_family = "wasm"))]
                if let Some(s) = slash.single() {
                    let music = audio.get(s);
                    am.play(music);
                }
                let rot = tr.0.rot;
                let v = tr.0.rot * Vec3::Y;
                let vel = v * (1.0 + player.velocity).min(MAX_VEL + 1.0);
                let pos = tr.0.pos + v * 0.5;

                cmd.spawn()
                    .insert_bundle(sprite_renderer::sprite_sheet_bundle(
                        sprites.bullet_sheet.clone(),
                        None,
                    ))
                    .insert_bundle((
                        LifeTime(Timer::new(Duration::from_secs(5), false)),
                        Bullet,
                        GameEntity,
                        UniformAnimation {
                            timer: Timer::new(Duration::from_millis(100), true),
                            n: sprites.bullet_n,
                        },
                        Velocity(vel.truncate()),
                        Cooldown(Timer::new(Duration::from_millis(200), false)),
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

fn setup_player(mut cmd: Commands, assets: Res<Sprites>) {
    // player
    spawn_player(cmd.spawn(), assets.player.clone());

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

fn spawn_player(cmd: &mut EntityCommands, sprite_handle: Handle<SpriteSheet>) {
    cmd.insert_bundle(transform::transform_bundle(
        transform::Transform::from_scale(Vec3::splat(0.5)),
    ))
    .insert_bundle(sprite_renderer::sprite_sheet_bundle(sprite_handle, None))
    .insert_bundle(aabb_bundle(
        AABB::around_origin(Vec2::splat(0.5)),
        PLAYER_TAG,
    ))
    .insert_bundle((
        GameEntity,
        Player::default(),
        Velocity::default(),
        RotationTime(Duration::default()),
    ));
}

fn setup_sprite_sheets(
    graphics_state: Res<GraphicsState>,
    mut assets: ResMut<assets::Assets<SpriteSheet>>,
    mut sprites: ResMut<Sprites>,
) {
    *sprites = Sprites {
        bullet_sheet: load_sprite_sheet(
            &graphics_state,
            include_bytes!("../assets/bullet.png"),
            Vec2::splat(128.0),
            2,
            "bullet",
            &mut assets,
        ),
        bullet_n: 2,
        asteroid_sheet: load_sprite_sheet(
            &graphics_state,
            include_bytes!("../assets/asteroids.png"),
            Vec2::splat(128.0),
            2,
            "asteroids",
            &mut assets,
        ),
        asteroid_n: 2,
        game_over_sheet: load_sprite_sheet(
            &graphics_state,
            include_bytes!("../assets/game_over.png"),
            Vec2::new(128.0, 32.0),
            1,
            "game_over",
            &mut assets,
        ),
        player: load_sprite_sheet(
            &graphics_state,
            include_bytes!("../assets/ship.png"),
            Vec2::new(32.0, 45.0),
            1,
            "ship",
            &mut assets,
        ),
        thrust_sheet: load_sprite_sheet(
            &graphics_state,
            include_bytes!("../assets/flame.png"),
            Vec2::splat(32.0),
            4,
            "flame",
            &mut assets,
        ),
        thrust_n: 4,
        digits: load_sprite_sheet(
            &graphics_state,
            include_bytes!("../assets/digits.png"),
            Vec2::splat(16.0),
            10,
            "digits",
            &mut assets,
        ),
    };
}

fn load_sprite_sheet(
    graphics_state: &GraphicsState,
    bytes: &[u8],
    box_size: Vec2,
    num_cols: u32,
    label: &str,
    assets: &mut Assets<SpriteSheet>,
) -> Handle<SpriteSheet> {
    let texture = renderer::texture::Texture::from_bytes(
        graphics_state.device(),
        graphics_state.queue(),
        bytes,
        label,
    )
    .unwrap();
    let sprite_sheet = SpriteSheet::from_texture(Vec2::ZERO, box_size, num_cols, texture);

    assets.insert(sprite_sheet)
}

fn restart_system(
    q_game_over: Query<&(), (With<GameOver>, WithOut<Cooldown>)>,
    mut cmd: Commands,
    assets: Res<Sprites>,
    inputs: Res<KeyBoardInputs>,
    mut score: ResMut<Score>,
    q_cleanup: Query<EntityId, With<GameEntity>>,
) {
    if q_game_over.single().is_some() {
        for key in inputs.just_released.iter() {
            if let VirtualKeyCode::Space = key {
                for id in q_cleanup.iter() {
                    cmd.delete(id);
                }
                spawn_player(cmd.spawn(), assets.player.clone());
                score.score.0 = 0;
            }
        }
    }
}

fn render_score(
    q_camera: Query<EntityId, With<PlayerCamera>>,
    q_scores: Query<EntityId, With<ScoreDigit>>,
    mut score: ResMut<Score>,
    mut cmd: Commands,
    assets: Res<Sprites>,
) {
    if score.score.0 == score.rendered_score {
        return;
    }
    for id in q_scores.iter() {
        cmd.delete(id);
    }
    score.rendered_score = score.score.0;
    let Some(camera_id) = q_camera.single() else {
        return;
    };

    // layouting
    let mut s = score.score.0;
    let mut digits = Vec::with_capacity(4); // TODO: smallvec
    digits.push(s % 10);
    s /= 10;
    while s > 0 {
        digits.push(s % 10);
        s /= 10;
    }
    let mut x_offset = -45.0;
    for digit in digits {
        transform::spawn_child(camera_id, &mut cmd, |cmd| {
            cmd.insert_bundle(transform_bundle(Transform::from_position(Vec3::new(
                x_offset, -45.0, -5.0,
            ))))
            .insert_bundle(sprite_sheet_bundle(
                assets.digits.clone(),
                SpriteInstance {
                    index: digit as u32,
                    flip: true,
                },
            ))
            .insert_bundle((ScoreDigit,));
        });
        x_offset += 1.0;
    }
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
            .add_system(restart_system)
            .add_system(cooldown_system)
            .add_system(render_score)
            .add_system(move_system);

        app.stage(Stage::PostUpdate).add_system(handle_collisions);

        app.add_startup_system(setup_sprite_sheets)
            .add_startup_system(setup_player.after(setup_sprite_sheets))
            .add_startup_system(setup_slash);

        app.insert_resource(Score {
            score: Wrapping(0),
            rendered_score: u64::MAX,
        });
        app.insert_resource(Sprites::default());
    }
}

pub async fn game() {
    let mut app = App::default();
    app.add_plugin(DefaultPlugins);
    app.add_plugin(GamePlugin);
    app.add_plugin(CollisionPlugin);
    app.run().await;
}
