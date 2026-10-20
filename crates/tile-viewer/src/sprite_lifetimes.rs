//! Viewer-clock expiry independent of publisher updates or game frames.
use std::{collections::HashMap, time::{Duration, Instant}};

#[derive(Default)]
pub(crate) struct SpriteLifetimes {
    highest: u64,
    deadlines: HashMap<u64, Instant>,
    expired: Vec<(u32, u32)>,
}

impl SpriteLifetimes {
    /// Observe lifetimes without mutating canonical scene sprites. Serials
    /// increase across all tellings in one session.
    pub(crate) fn update(&mut self, scene: &scene::Scene, now: Instant) -> bool {
        let before = self.highest;
        for life in &scene.sprite_lifetimes {
            if life.serial > before {
                self.deadlines.entry(life.serial).or_insert(now + Duration::from_millis(life.duration_ms.into()));
                self.highest = self.highest.max(life.serial);
            }
        }
        self.deadlines.retain(|serial, _| scene.sprite_lifetimes.iter().any(|life| life.serial == *serial));
        let expired = scene.sprite_lifetimes.iter().filter(|life|
            // A previously removed serial is already spent. Replaying an
            // old scene must not resurrect its transient sprites.
            !self.deadlines.get(&life.serial).is_some_and(|&until| now < until)
        ).map(|life| (life.sprites.0.min(scene.sprites.len() as u32), life.sprites.1.min(scene.sprites.len() as u32))).collect::<Vec<_>>();
        let changed = expired != self.expired;
        self.expired = expired;
        changed
    }

    /// Apply only to a derived render list. Preserve canonical clips so a
    /// lifetime-only update with a new serial can show identical art again.
    pub(crate) fn clip(&self, index: usize, original: Option<[i32; 4]>) -> Option<[i32; 4]> {
        if self.expired.iter().any(|&(start, end)| (start as usize..end as usize).contains(&index)) {
            Some([0; 4])
        } else { original }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sprites() -> scene::Scene {
        let mut scene = scene::Scene::default();
        scene.sprites = vec![scene::FreeSprite::default(); 3];
        scene.sprite_lifetimes = vec![scene::SpriteLifetime { serial: 1, sprites: (1, 2), duration_ms: 2000 }];
        scene
    }
    fn clips(clock: &SpriteLifetimes, scene: &scene::Scene) -> Vec<Option<[i32; 4]>> {
        scene.sprites.iter().enumerate().map(|(i, s)| clock.clip(i, s.clip)).collect()
    }
    #[test]
    fn expires_without_any_incoming_scene_and_keeps_unrelated_sprites() {
        let now = Instant::now();
        let mut clock = SpriteLifetimes::default();
        let scene = sprites();
        assert!(!clock.update(&scene, now));
        assert!(!clock.update(&scene, now + Duration::from_millis(1999)));
        assert!(clock.update(&scene, now + Duration::from_secs(2)));
        assert_eq!(clips(&clock, &scene), vec![None, Some([0; 4]), None]);
        assert!(scene.sprites.iter().all(|s| s.clip.is_none()), "canonical scene stays intact");
        // A replacement scene with the same serial cannot renew its life.
        assert!(!clock.update(&sprites(), now + Duration::from_secs(3)));
        assert_eq!(clips(&clock, &scene)[1], Some([0; 4]));
        // Nor can a serial return after a scene without it.
        clock.update(&scene::Scene::default(), now + Duration::from_secs(4));
        clock.update(&sprites(), now + Duration::from_secs(5));
        assert_eq!(clips(&clock, &scene)[1], Some([0; 4]));
    }
    #[test]
    fn lifetime_only_update_renews_identical_art_and_restores_original_clip() {
        let now = Instant::now();
        let mut clock = SpriteLifetimes::default();
        let mut scene = sprites();
        scene.sprites[1].clip = Some([10, 20, 30, 40]);
        clock.update(&scene, now);
        clock.update(&scene, now + Duration::from_secs(2));
        assert_eq!(clips(&clock, &scene)[1], Some([0; 4]));
        // This update carries no replacement sprites at all.
        scene.apply(scene::Update::SpriteLifetimes(vec![scene::SpriteLifetime {
            serial: 2, sprites: (1, 2), duration_ms: 2000,
        }]));
        assert!(clock.update(&scene, now + Duration::from_secs(3)));
        assert_eq!(clips(&clock, &scene)[1], Some([10, 20, 30, 40]));
        assert!(clock.update(&scene, now + Duration::from_secs(5)));
        assert_eq!(clips(&clock, &scene)[1], Some([0; 4]));
    }
}
