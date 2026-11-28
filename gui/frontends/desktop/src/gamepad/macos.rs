//! Shared macOS controller enumeration: snapshot the GameController list
//! and hand the first match to the caller. Both peripheral readers poll
//! this way, so hotplug needs no bookkeeping anywhere and the unsafe
//! snapshot block lives in exactly one place.

use objc2_game_controller::GCController;

pub(crate) fn first_controller<T>(mut pick: impl FnMut(&GCController) -> Option<T>) -> Option<T> {
    // SAFETY: snapshot getter; the retained array outlives this scope and
    // each controller is only read, never stored.
    unsafe {
        let controllers = GCController::controllers();
        for index in 0..controllers.count() {
            let controller = controllers.objectAtIndex(index);
            if let Some(found) = pick(&controller) {
                return Some(found);
            }
        }
        None
    }
}
