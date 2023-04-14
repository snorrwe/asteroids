use engine::cecs::{
    prelude::{Bundle, EntityId, ResMut},
    query::Query,
    systems::IntoSystem,
};
use engine::glam::Vec2;

use crate::{transform::GlobalTransform, Plugin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionTag {
    /// src is the tag of the object itself (should be 1 bit per class)
    pub src: u8,
    /// dst is the tags of the objects this can collide with
    pub dst: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AABB {
    pub min: Vec2,
    pub max: Vec2,
}

impl AABB {
    pub fn around_origin(size: Vec2) -> Self {
        let extents = size / 2.0;
        Self {
            min: -extents,
            max: extents,
        }
    }
}

pub fn test_aabb_aabb(a: &AABB, b: &AABB) -> bool {
    for i in 0..2 {
        if a.max[i] < b.min[i] || b.max[i] < a.min[i] {
            return false;
        }
    }
    true
}

struct AABBBuffer(pub Vec<(EntityId, AABB, CollisionTag)>);

struct GlobalAABB(AABB);

fn update_aabbs_system(mut q: Query<(&mut GlobalAABB, &GlobalTransform, &AABB)>) {
    q.par_for_each_mut(|(out, tr, aabb)| {
        let p = (aabb.max + aabb.min) * 0.5;
        let size = aabb.max - aabb.min;
        let size = tr.0.scale.truncate() * size * 0.5;
        let offset = tr.0.pos.truncate();
        out.0 = AABB {
            min: p - size + offset,
            max: p + size + offset,
        };
    });
}

fn collect_aabbs_system(
    mut buff: ResMut<AABBBuffer>,
    q: Query<(EntityId, &GlobalAABB, &CollisionTag)>,
) {
    buff.0.clear();
    for (id, aabb, tag) in q.iter() {
        buff.0.push((id, aabb.0, *tag));
    }
}

struct SortAxis(usize);

#[derive(Default)]
pub struct Collisions(pub Vec<CollisionEvent>);

#[derive(Debug)]
pub struct CollisionEvent {
    pub entity_1: EntityId,
    pub tag1: CollisionTag,
    pub entity_2: EntityId,
    pub tag2: CollisionTag,
}

fn sort_sweep_system(
    mut buff: ResMut<AABBBuffer>,
    mut axis: ResMut<SortAxis>,
    mut collisions: ResMut<Collisions>,
) {
    collisions.0.clear();
    let sort_axis = axis.0;
    buff.0.sort_unstable_by(|a, b| {
        a.1.min[sort_axis]
            .partial_cmp(&b.1.min[sort_axis])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut sum = Vec2::ZERO;
    let mut sum2 = Vec2::ZERO;

    for (i, a) in buff.0.iter().enumerate() {
        for b in &buff.0[i + 1..] {
            if b.1.min[sort_axis] > a.1.max[sort_axis] {
                break;
            }
            // tags are not commutative
            if ((a.2.src & b.2.dst != 0) || (b.2.src & a.2.dst != 0)) && test_aabb_aabb(&a.1, &b.1)
            {
                collisions.0.push(CollisionEvent {
                    entity_1: a.0,
                    tag1: a.2,
                    entity_2: b.0,
                    tag2: b.2,
                });
            }
        }
        let p = (a.1.max + a.1.min) / 2.0;
        sum += p;
        sum2 += p * p;
    }

    let variance = sum2 - ((sum * sum) / buff.0.len() as f32);
    if variance.y > variance.x {
        axis.0 = 1;
    } else {
        axis.0 = 0;
    }
}

pub struct CollisionPlugin;

impl Plugin for CollisionPlugin {
    fn build(self, app: &mut crate::App) {
        app.stage(crate::Stage::Update)
            .add_system(update_aabbs_system)
            .add_system(collect_aabbs_system.after(update_aabbs_system))
            .add_system(sort_sweep_system.after(collect_aabbs_system));

        app.insert_resource(AABBBuffer(Vec::default()));
        app.insert_resource(SortAxis(0));
        app.insert_resource(Collisions::default());
    }
}

pub fn aabb_bundle(aabb: AABB, tag: CollisionTag) -> impl Bundle {
    (aabb, GlobalAABB(aabb), tag)
}
