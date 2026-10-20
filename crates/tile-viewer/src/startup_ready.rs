//! Optional acknowledgement to a launcher, after a complete title menu presents.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

static SENT: AtomicBool = AtomicBool::new(false);

pub(crate) fn presented(scene: &scene::Scene, art_matches: bool, drawable_menu: bool) {
    if SENT.load(Ordering::Relaxed) || !drawable_menu || !complete_menu(scene, art_matches) {
        return;
    }
    let Some(path) = std::env::var_os("CIV3R_STARTUP_READY_FILE").map(PathBuf::from) else {
        return;
    };
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&temporary, b"menu-presented\n")
        .and_then(|()| std::fs::rename(&temporary, &path))
        .is_ok()
    {
        SENT.store(true, Ordering::Relaxed);
    } else {
        let _ = std::fs::remove_file(temporary);
    }
}

fn complete_menu(scene: &scene::Scene, art_matches: bool) -> bool {
    art_matches
        && scene.seq != 0
        && scene.interface.menu_fit
        && scene
            .views
            .iter()
            .any(|view| view.view.interface_only && !view.overlay.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loading_and_unmatched_art_do_not_acknowledge_readiness() {
        let mut scene = scene::Scene::default();
        assert!(!complete_menu(&scene, true));
        scene.seq = 1;
        scene.interface.menu_fit = true;
        assert!(!complete_menu(&scene, true));
        scene.views.push(scene::ViewState {
            view: scene::View {
                menu: true,
                interface_only: true,
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(!complete_menu(&scene, false));
        assert!(!complete_menu(&scene, true));
        scene.views[0].overlay.push(scene::FreeSprite::default());
        assert!(complete_menu(&scene, true));
    }
}
