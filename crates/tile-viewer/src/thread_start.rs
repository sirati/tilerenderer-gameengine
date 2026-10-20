//! Optional host setup runs before any work on a viewer-owned native thread.
#![forbid(unsafe_code)]

pub(crate) fn initialized<T>(initialize: Option<fn()>, work: impl FnOnce() -> T) -> T {
    if let Some(initialize) = initialize {
        initialize();
    }
    work()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    thread_local! { static INITIALIZED: Cell<bool> = const { Cell::new(false) }; }
    fn initialize() {
        INITIALIZED.with(|state| assert!(!state.replace(true)));
    }

    #[test]
    fn host_initialization_precedes_native_work_on_its_own_thread() {
        let caller = std::thread::current().id();
        let owner = std::thread::spawn(|| {
            initialized(Some(initialize), || {
                INITIALIZED.with(|state| assert!(state.get()));
                std::thread::current().id()
            })
        })
        .join()
        .unwrap();
        assert_ne!(caller, owner);
        INITIALIZED.with(|state| assert!(!state.get(), "the caller's context is unchanged"));
    }

    #[test]
    fn standalone_work_has_no_implicit_host_initialization() {
        std::thread::spawn(|| {
            initialized(None, || {
                INITIALIZED.with(|state| assert!(!state.get()));
            })
        })
        .join()
        .unwrap();
    }
}
