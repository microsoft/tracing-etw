use core::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
};
use std::{panic::catch_unwind, sync::Barrier, thread, vec::Vec};

use super::LocalKey;

#[test]
fn isolates_threads_reuses_values_and_runs_destructors() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Tracked(usize);
    impl Drop for Tracked {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    static LOCAL: LocalKey<RefCell<Tracked>> = LocalKey::new(|| RefCell::new(Tracked(0)));

    let barrier = Barrier::new(9);
    thread::scope(|scope| {
        let mut handles = Vec::new();
        for index in 1..=8 {
            let barrier = &barrier;
            handles.push(scope.spawn(move || {
                barrier.wait();
                LOCAL.with_borrow(|value| assert_eq!(value.0, 0));
                LOCAL.with_borrow_mut(|value| value.0 = index);
                LOCAL.with_borrow(|value| assert_eq!(value.0, index));
            }));
        }
        barrier.wait();
        LOCAL.with_borrow_mut(|value| value.0 = 42);
        // Scope completion alone can precede native TLS destruction.
        for handle in handles {
            handle.join().unwrap();
        }
    });
    LOCAL.with_borrow(|value| assert_eq!(value.0, 42));
    assert_eq!(DROPS.load(Ordering::SeqCst), 8);
}

#[test]
fn rejects_recursive_initialization_and_retries_after_unwinding() {
    static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
    static LOCAL: LocalKey<RefCell<usize>> = LocalKey::new(|| {
        if ATTEMPTS.fetch_add(1, Ordering::SeqCst) == 0 {
            LOCAL.with_borrow(|_| {});
        }
        RefCell::new(7)
    });
    assert!(catch_unwind(|| LOCAL.with_borrow(|_| {})).is_err());
    LOCAL.with_borrow(|value| assert_eq!(*value, 7));
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);
}

#[test]
fn rejects_reentrant_mutable_borrows_and_recovers_after_unwinding() {
    static LOCAL: LocalKey<RefCell<usize>> = LocalKey::new(|| RefCell::new(0));
    assert!(catch_unwind(|| LOCAL.with_borrow_mut(|_| LOCAL.with_borrow(|_| {}))).is_err());
    LOCAL.with_borrow_mut(|value| *value = 9);
    LOCAL.with_borrow(|value| assert_eq!(*value, 9));
}

#[test]
fn rejects_access_to_a_value_while_it_is_being_destroyed() {
    static REJECTED: AtomicUsize = AtomicUsize::new(0);
    struct Reenter;
    impl Drop for Reenter {
        fn drop(&mut self) {
            if catch_unwind(|| LOCAL.with(|_| {})).is_err() {
                REJECTED.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    static LOCAL: LocalKey<Reenter> = LocalKey::new(|| Reenter);
    thread::spawn(|| LOCAL.with(|_| {})).join().unwrap();
    assert_eq!(REJECTED.load(Ordering::SeqCst), 1);
}

// #[cfg(windows)]
// #[test]
// fn native_slot_operations_preserve_the_windows_last_error() {
//     use windows_sys::Win32::Foundation::{GetLastError, SetLastError};

//     // SAFETY: These APIs read/write only this test thread's error code.
//     static LOCAL: LocalKey<u32> = LocalKey::new(|| unsafe { GetLastError() });
//     unsafe { SetLastError(12345) };
//     let initial_error = LOCAL.with(|value| *value);
//     assert_eq!(unsafe { GetLastError() }, 12345);
//     assert_eq!(initial_error, 12345);
//     unsafe { SetLastError(54321) };
//     LOCAL.with(|_| {});
//     assert_eq!(unsafe { GetLastError() }, 54321);
// }

#[cfg(windows)]
#[test]
fn isolates_fibers_and_preserves_the_callers_value_when_deleting_another_fiber() {
    use core::{ffi::c_void, ptr};
    use windows_sys::Win32::System::Threading::{
        ConvertFiberToThread, ConvertThreadToFiber, CreateFiber, DeleteFiber, SwitchToFiber,
    };

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Tracked(usize);
    impl Drop for Tracked {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    static LOCAL: LocalKey<RefCell<Tracked>> = LocalKey::new(|| RefCell::new(Tracked(0)));

    struct Context {
        parent: *mut c_void,
        initial: usize,
        updated: usize,
    }
    unsafe extern "system" fn run(data: *mut c_void) {
        // SAFETY: The context lives on the parent stack until this fiber is deleted.
        let context = unsafe { &mut *data.cast::<Context>() };
        context.initial = LOCAL.with_borrow(|value| value.0);
        LOCAL.with_borrow_mut(|value| value.0 = 7);
        context.updated = LOCAL.with_borrow(|value| value.0);
        // SAFETY: The parent fiber is suspended in SwitchToFiber and remains live.
        unsafe { SwitchToFiber(context.parent) };
    }
    struct Cleanup(*mut c_void);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            // SAFETY: The child, if present, is suspended; the current thread
            // was converted to a fiber and must be restored before returning.
            unsafe {
                if !self.0.is_null() {
                    DeleteFiber(self.0);
                }
                assert_ne!(ConvertFiberToThread(), 0);
            }
        }
    }

    // SAFETY: This test owns its thread and has not converted it to a fiber.
    let parent = unsafe { ConvertThreadToFiber(ptr::null()) };
    assert!(!parent.is_null());
    let mut cleanup = Cleanup(ptr::null_mut());
    let mut context = Context {
        parent,
        initial: usize::MAX,
        updated: usize::MAX,
    };
    LOCAL.with_borrow_mut(|value| value.0 = 42);
    // SAFETY: Context stays live until the suspended child has been deleted.
    cleanup.0 = unsafe { CreateFiber(0, Some(run), ptr::from_mut(&mut context).cast()) };
    assert!(!cleanup.0.is_null());
    // SAFETY: The child was just created with a valid entry point and context.
    unsafe { SwitchToFiber(cleanup.0) };
    assert_eq!((context.initial, context.updated), (0, 7));
    LOCAL.with_borrow(|value| assert_eq!(value.0, 42));
    // SAFETY: The child has switched back to this fiber and is suspended.
    unsafe { DeleteFiber(cleanup.0) };
    cleanup.0 = ptr::null_mut();
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    LOCAL.with_borrow(|value| assert_eq!(value.0, 42));
}
