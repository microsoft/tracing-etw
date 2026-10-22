use super::RwLock;
use core::ops::Deref;

enum LazyState<T> {
    Uninitialized(fn() -> T),
    Poisoned,
    Initialized(T),
}

pub(crate) struct LazyLock<T> {
    state: RwLock<LazyState<T>>,
}

impl<T> LazyLock<T> {
    pub(crate) const fn new(initialize: fn() -> T) -> Self {
        Self {
            state: RwLock::new(LazyState::Uninitialized(initialize)),
        }
    }

    fn initialize(&self) -> *const T {
        let mut state = self.state.write();
        match &*state {
            LazyState::Uninitialized(initialize) => {
                let initialize = *initialize;
                *state = LazyState::Poisoned;
                *state = LazyState::Initialized(initialize());
            }
            LazyState::Poisoned => panic!("LazyLock initializer panicked"),
            LazyState::Initialized(_) => {}
        }
        match &*state {
            LazyState::Initialized(value) => value,
            _ => unreachable!("LazyLock initialization did not complete"),
        }
    }
}

impl<T> Deref for LazyLock<T> {
    type Target = T;

    fn deref(&self) -> &T {
        let state = self.state.read();
        let value = match &*state {
            LazyState::Initialized(value) => value as *const T,
            _ => {
                drop(state);
                self.initialize()
            }
        };
        // SAFETY: Initialization completed under the lock. An initialized state
        // is never mutated again, and self keeps the returned value alive.
        unsafe { &*value }
    }
}
