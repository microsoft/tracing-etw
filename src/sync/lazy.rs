use super::RwLock;
use core::{
    cell::UnsafeCell,
    ops::Deref,
    sync::atomic::{AtomicBool, Ordering},
};

enum LazyState<T> {
    Uninitialized(fn() -> T),
    Poisoned,
    Initialized(T),
}

pub(crate) struct LazyLock<T> {
    initialized: AtomicBool,
    initialization: RwLock<()>,
    state: UnsafeCell<LazyState<T>>,
}

// SAFETY: Initialization is exclusive, and Release/Acquire publication makes
// the completed state visible to readers. Published values are never mutated.
unsafe impl<T: Send + Sync> Sync for LazyLock<T> {}

impl<T> LazyLock<T> {
    pub(crate) const fn new(initialize: fn() -> T) -> Self {
        Self {
            initialized: AtomicBool::new(false),
            initialization: RwLock::new(()),
            state: UnsafeCell::new(LazyState::Uninitialized(initialize)),
        }
    }

    #[cold]
    fn initialize(&self) {
        let _guard = self.initialization.write();
        // Recheck before taking a mutable reference: another thread may have
        // published the value and returned shared references while we waited.
        if self.initialized.load(Ordering::Acquire) {
            return;
        }
        {
            // SAFETY: The lock excludes other initializers, and no readers can
            // access the state until initialization has been published.
            let state = unsafe { &mut *self.state.get() };
            match state {
                LazyState::Uninitialized(initialize) => {
                    let initialize = *initialize;
                    *state = LazyState::Poisoned;
                    *state = LazyState::Initialized(initialize());
                }
                LazyState::Poisoned => panic!("LazyLock initializer panicked"),
                LazyState::Initialized(_) => unreachable!("Lazy value was not published"),
            }
        }
        self.initialized.store(true, Ordering::Release);
    }
}

impl<T> Deref for LazyLock<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        if !self.initialized.load(Ordering::Acquire) {
            self.initialize();
        }
        // SAFETY: Publication makes the immutable state visible. Access remains
        // relative to self, so moving an initialized LazyLock is safe.
        match unsafe { &*self.state.get() } {
            LazyState::Initialized(value) => value,
            _ => unreachable!("LazyLock initialization did not complete"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread, time::Duration};

    #[test]
    fn initialized_reads_bypass_the_initialization_lock() {
        let lazy = LazyLock::new(|| 42);
        assert_eq!(*lazy, 42);
        let guard = lazy.initialization.write();
        let (sender, receiver) = mpsc::channel();
        thread::scope(|scope| {
            let reader = scope.spawn(|| sender.send(*lazy).unwrap());
            let result = receiver.recv_timeout(Duration::from_secs(5));
            drop(guard);
            reader.join().unwrap();
            assert_eq!(result.unwrap(), 42);
        });
    }
}
