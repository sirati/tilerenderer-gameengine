use super::*;
use scene::Scene;

/// The first queued report's button is a target of the Alt mode, where the
/// report is shown, and a click on it there reads the report (nothing
/// reaches the driver but `CloseQueued`).
#[test]
fn the_first_queued_reports_button_is_an_alt_target_and_its_click_reads_it() {
    let scene = Scene::default();
    {
        let mut layout = Layout::default();
        layout.set_queued_shown(vec![
            crate::scaling::QueuedShown { serial: 4, view: 0x1101, rect: [1200, 16, 376, 297], button: Some([1530, 279, 24, 24]) },
            crate::scaling::QueuedShown { serial: 5, view: 0x1100, rect: [1210, 26, 376, 297], button: Some([1540, 289, 24, 24]) },
        ]);
        let targets = SnapInput::targets(&scene, &layout);
        let target = targets.iter().find(|t| t.id == QUEUED_ID).expect("the queued report's target");
        assert_eq!(target.rect, [1530.0, 279.0, 24.0, 24.0], "the first one's button only");
        assert_eq!(targets.iter().filter(|t| t.id == QUEUED_ID - 1).count(), 0);
        let at = target.center();
        let mut snap = SnapInput::default();
        let out = snap.perform(vec![Action::Button { button: Button::Left, down: true, at }, Action::Button { button: Button::Left, down: false, at }], &mut layout);
        assert_eq!(out.iter().filter(|i| matches!(i, Input::CloseQueued { .. } | Input::MouseButton { .. })).cloned().collect::<Vec<_>>(), vec![Input::CloseQueued { serial: 4 }], "{out:?}");
        assert_eq!(layout.queued_shown().iter().map(|q| q.serial).collect::<Vec<_>>(), [5]);
    }
}
