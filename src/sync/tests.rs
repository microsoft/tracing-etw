use super::*;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::{
    boxed::Box,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Barrier, mpsc},
    thread,
    time::Duration,
};

#[test]
fn permits_simultaneous_readers() {
    let lock = RwLock::new(42);
    thread::scope(|scope| {
        let first = lock.read();
        let (sender, receiver) = mpsc::channel();
        let lock = &lock;
        let reader = scope.spawn(move || {
            let second = lock.read();
            sender.send(*second).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(5));
        drop(first);
        reader.join().unwrap();
        assert_eq!(result.unwrap(), 42);
    });
}

#[test]
fn serializes_contended_writes_and_releases_after_panics() {
    let lock = RwLock::new(0);
    let barrier = Barrier::new(8);
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..128 {
                    *lock.write() += 1;
                }
            });
        }
    });
    assert_eq!(*lock.read(), 1024);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            let mut guard = lock.write();
            *guard = 7;
            panic!("test panic while holding exclusive access");
        }))
        .is_err()
    );
    assert_eq!(*lock.read(), 7);
    *lock.write() = 8;
    assert_eq!(*lock.read(), 8);
}

#[test]
fn initializes_lazy_values_once_under_contention() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let lazy = LazyLock::new(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        std::string::String::from("initialized")
    });
    let barrier = Barrier::new(8);
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                let value = &*lazy;
                for _ in 0..128 {
                    assert_eq!(value, "initialized");
                    assert!(core::ptr::eq(value, &*lazy));
                }
            });
        }
    });
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_lazy_initialization_remains_poisoned() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let lazy = LazyLock::<u64>::new(|| {
        CALLS.fetch_add(1, Ordering::SeqCst);
        panic!("test initializer failure");
    });
    let barrier = Barrier::new(8);
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                assert!(catch_unwind(AssertUnwindSafe(|| *lazy)).is_err());
            });
        }
    });
    assert!(catch_unwind(AssertUnwindSafe(|| *lazy)).is_err());
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn initialized_lazy_values_survive_moves() {
    let mut values = [LazyLock::new(|| 42), LazyLock::new(|| 99)];
    assert_eq!(*values[0], 42);
    assert_eq!(*values[1], 99);
    values.swap(0, 1);
    assert_eq!(*values[0], 99);
    assert_eq!(*values[1], 42);
}

#[test]
fn lazy_values_drop_once_without_initializing_on_drop() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Value;
    impl Drop for Value {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn initialize() -> Value {
        CALLS.fetch_add(1, Ordering::SeqCst);
        Value
    }

    drop(LazyLock::new(initialize));
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);

    let lazy = LazyLock::new(initialize);
    let _ = &*lazy;
    let moved = Box::new(lazy);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    drop(moved);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}
