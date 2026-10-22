#![no_std]
#![no_main]

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("The no_std consumer requires Windows or Linux.");

mod allocator;

use core::{ffi::c_void, fmt::Write, ptr};
use trace::{Level, Subscriber, callsite::Callsite, metadata::Kind, span::Attributes};

// This also makes any transitive Rust std dependency a duplicate-panic-handler
// compilation error. The executable uses native C startup and allocation only.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    // Diagnostics are best-effort; stderr failure must not prevent termination.
    let _ = writeln!(Stderr, "{info}");
    // SAFETY: abort terminates the process without unwinding through native code.
    unsafe { libc::abort() }
}

struct Stderr;

impl Write for Stderr {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let mut bytes = text.as_bytes();
        while !bytes.is_empty() {
            #[cfg(windows)]
            let count = bytes.len().try_into().map_err(|_| core::fmt::Error)?;
            #[cfg(target_os = "linux")]
            let count = bytes.len();
            // SAFETY: The buffer is valid for count bytes; fd 2 is native stderr.
            let written = unsafe { libc::write(2, bytes.as_ptr().cast(), count) };
            if written <= 0 {
                return Err(core::fmt::Error);
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }
}

fn exercise_subscriber(subscriber: impl Subscriber) {
    let metadata = trace::callsite! {
        name: "NoStdSpan",
        kind: Kind::SPAN,
        target: module_path!(),
        level: Level::INFO,
        fields: value
    }
    .metadata();
    let span = subscriber.new_span(&Attributes::new_root(
        metadata,
        &trace::valueset!(metadata.fields(), value = 1u64),
    ));
    assert!(subscriber.current_span().id().is_none());
    subscriber.enter(&span);
    assert_eq!(subscriber.current_span().id(), Some(&span));
    let cloned = subscriber.clone_span(&span);
    assert!(!subscriber.try_close(cloned));
    let child = subscriber.new_span(&Attributes::new(
        metadata,
        &trace::valueset!(metadata.fields(), value = 2u64),
    ));

    let event_metadata = trace::callsite! {
        name: "NoStdEvent",
        kind: Kind::EVENT,
        target: module_path!(),
        level: Level::INFO,
        fields: value, message
    }
    .metadata();
    // Direct dispatch exercises native event builders even without a collector.
    subscriber.event(&trace::Event::new(
        event_metadata,
        &trace::valueset!(
            event_metadata.fields(),
            value = 42u64,
            message = format_args!("no_std {}", 42)
        ),
    ));
    subscriber.exit(&span);
    assert!(subscriber.current_span().id().is_none());
    assert!(!subscriber.try_close(span));
    assert!(subscriber.try_close(child));

    // Leave the final reference entered so native thread-exit cleanup runs.
    let entered = subscriber.new_span(&Attributes::new_root(
        metadata,
        &trace::valueset!(metadata.fields(), value = 3u64),
    ));
    subscriber.enter(&entered);
    assert!(!subscriber.try_close(entered));
}

fn exercise() {
    exercise_subscriber(exporter::LayerBuilder::new("NoStdNormal").build().unwrap());
    exercise_subscriber(
        exporter::LayerBuilder::new_common_schema_events("NoStdCommonSchema")
            .build_subscriber()
            .unwrap(),
    );
    // The fixture deliberately has no direct paste/tracing-core dependencies,
    // and renames both public crates to check exported-macro hygiene.
    exporter::etw_event!(name: "NoStdMacro", Level::INFO, 1, value = 42u64);
}

#[cfg(target_os = "linux")]
extern "C" fn worker(_: *mut c_void) -> *mut c_void {
    exercise();
    ptr::null_mut()
}

#[cfg(windows)]
extern "system" fn worker(_: *mut c_void) -> u32 {
    exercise();
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    #[cfg(windows)]
    let group = exporter::ProviderGroupId::from_u128(&0x2bd01356_123d_45bb_b99b_9d7ec39e4954);
    #[cfg(target_os = "linux")]
    let group = exporter::ProviderGroupId::Borrowed("nostdgroup");
    let subscriber: exporter::EtwLayer<(), exporter::NormalOutput> =
        exporter::LayerBuilder::new("NoStdGlobal")
            .with_provider_id(exporter::GuidWrapper::from(
                0x2bd01356_123d_45bb_b99b_9d7ec39e4954,
            ))
            .with_provider_group(group)
            .build()
            .unwrap();
    trace::subscriber::set_global_default(subscriber).unwrap();
    #[cfg(target_os = "linux")]
    {
        let mut thread = core::mem::MaybeUninit::uninit();
        // SAFETY: The output pointer is writable and worker has the pthread ABI.
        let result = unsafe {
            libc::pthread_create(thread.as_mut_ptr(), ptr::null(), worker, ptr::null_mut())
        };
        assert_eq!(result, 0, "pthread_create failed: {result}");
        // SAFETY: Successful creation initialized the joinable thread handle.
        let result = unsafe { libc::pthread_join(thread.assume_init(), ptr::null_mut()) };
        assert_eq!(result, 0, "pthread_join failed: {result}");
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{CreateThread, INFINITE, WaitForSingleObject},
        };
        // SAFETY: worker has the native ABI, uses no borrowed context, and the
        // returned thread is joined before this executable exits.
        unsafe {
            let thread = CreateThread(
                ptr::null(),
                0,
                Some(worker),
                ptr::null(),
                0,
                ptr::null_mut(),
            );
            assert!(!thread.is_null(), "CreateThread failed");
            let result = WaitForSingleObject(thread, INFINITE);
            let closed = CloseHandle(thread);
            assert_eq!(
                result, WAIT_OBJECT_0,
                "WaitForSingleObject failed: {result}"
            );
            assert_ne!(closed, 0, "CloseHandle failed");
        }
    }
    0
}
