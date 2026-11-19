//! Public prepared-memory contracts and an isolated hot-path allocation count.

use effect::memory::{
    Admission, Bytes, CapacityUnit, InitializationError, MemoryError, MemoryPlan, MemoryPolicy,
    Partition, PreparedMemory, RegionId, ResourceCapacity,
};
use effect::{effect_drop_guard, memory_vec};
use std::alloc::{Allocator, GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::future::{Future, pending};
use std::io;
use std::marker::PhantomPinned;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::task::{Context, Poll, Waker};

const REGION: RegionId = RegionId::named("validation");

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

struct LocalCounter;

// SAFETY: forwards the exact allocator contracts to System. The counter is
// const-initialized thread-local storage and invokes no allocator itself.
unsafe impl GlobalAlloc for LocalCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: GlobalAlloc caller supplied the same valid allocation layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: GlobalAlloc caller supplied the same valid allocation layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller supplies a live System pointer and its layout.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        // SAFETY: the pointer, prior layout and requested size are forwarded unchanged.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: LocalCounter = LocalCounter;

fn count() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(allocations) = count.get() {
            count.set(Some(allocations.saturating_add(1)));
        }
    });
}

fn arena(size: usize, policy: MemoryPolicy) -> Result<PreparedMemory<'static>, MemoryError> {
    let size = Bytes::new(size);
    let regions = [Partition {
        id: REGION,
        capacity: size,
    }];
    MemoryPlan::new(size, &regions)?
        .metadata(REGION)?
        .alignment(64)?
        .policy(policy)
        .prepare()
}

#[test]
fn exact_exhaustion_charges_padding_and_reuses_the_range() -> Result<(), Box<dyn std::error::Error>>
{
    let prepared = arena(4096, MemoryPolicy::BoundedAllocation)?;
    let allocator = prepared.allocator(REGION)?;
    let original = prepared.usage(REGION)?;
    let empty = allocator.allocate_bytes(Layout::from_size_align(0, 64)?)?;
    assert!(empty.is_empty());
    assert_eq!(prepared.usage(REGION)?.active_allocations, 0);
    let region_bytes = original.committed.get();
    assert!(matches!(
        allocator.allocate_bytes(Layout::from_size_align(region_bytes, 64)?),
        Err(MemoryError::Exhausted { .. })
    ));
    assert!(matches!(
        allocator.allocate_bytes(Layout::from_size_align(1, 128)?),
        Err(MemoryError::UnsupportedAlignment {
            requested: 128,
            supported: 64
        })
    ));
    assert!(
        allocator
            .allocate_bytes(Layout::from_size_align(isize::MAX.cast_unsigned(), 1)?)
            .is_err()
    );
    assert!(matches!(
        Bytes::new(usize::MAX).checked_add(Bytes::new(1)),
        Err(MemoryError::ArithmeticOverflow)
    ));

    let (mut fits, mut refuses) = (0, region_bytes);
    while refuses - fits > 1 {
        let size = fits.midpoint(refuses);
        if allocator
            .allocate_bytes(Layout::from_size_align(size, 64)?)
            .is_ok()
        {
            fits = size;
        } else {
            refuses = size;
        }
    }
    assert!(fits > 0);
    let layout = Layout::from_size_align(fits, 64)?;
    let mut full = allocator.allocate_bytes(layout)?;
    let address = full.as_ptr().addr();
    assert_eq!(address % 64, 0);
    assert!(full.iter().all(|byte| *byte == 0));
    full.fill(0xa5);
    let used = prepared.usage(REGION)?;
    assert_eq!(used.active_payload, Bytes::new(fits));
    assert!(used.active_backing > used.active_payload);
    assert!(allocator.allocate_bytes(Layout::new::<u8>()).is_err());
    drop(full);
    assert_eq!(
        prepared.usage(REGION)?.active_backing,
        original.active_backing
    );
    let memory = prepared.seal();
    assert!(matches!(
        allocator.allocate_bytes(layout),
        Err(MemoryError::Sealed)
    ));
    let mut recovered = memory.allocate(REGION, layout)?;
    assert_eq!(recovered.as_ptr().addr(), address);
    assert!(recovered.iter().all(|byte| *byte == 0));
    drop(allocator);
    drop(memory);
    recovered.fill(7);
    assert!(recovered.iter().all(|byte| *byte == 7));
    Ok(())
}

#[derive(Debug)]
struct CountDrop(Arc<AtomicUsize>);
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn partial_initialization_rollback_includes_error_and_unwind() -> Result<(), MemoryError> {
    let prepared = arena(4096, MemoryPolicy::PreparedReuse)?;
    let drops = Arc::new(AtomicUsize::new(0));
    let before = prepared.usage(REGION)?.active_backing;
    let construction_drops = drops.clone();
    let failed = memory_vec! {
        allocator: prepared.allocator(REGION)?, capacity: 4,
        init |index| {
            if index == 2 { return Err("failure"); }
            Ok(CountDrop(construction_drops.clone()))
        },
    };
    assert!(matches!(
        failed,
        Err(InitializationError::Initialize("failure"))
    ));
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!(prepared.usage(REGION)?.active_backing, before);
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        let construction_drops = drops.clone();
        let _ = memory_vec! {
            allocator: prepared.allocator(REGION)?, capacity: 4,
            init |index| {
                assert_ne!(index, 2, "constructor unwind");
                Ok::<_, !>(CountDrop(construction_drops.clone()))
            },
        };
        Ok::<_, MemoryError>(())
    }));
    assert!(unwound.is_err());
    assert_eq!(drops.load(Ordering::Relaxed), 4);
    assert_eq!(prepared.usage(REGION)?.active_backing, before);
    Ok(())
}

#[test]
fn allocator_provenance_survives_raw_parts_and_sealed_resize_refusal() -> Result<(), MemoryError> {
    let prepared = arena(4096, MemoryPolicy::PreparedReuse)?;
    let allocator = prepared.allocator(REGION)?;
    let mut data = allocator.try_clone()?.vector::<u64>(4)?;
    data.extend([17, 29]);
    let address = data.as_ptr().addr();
    let (pointer, length, capacity, actual_allocator) = data.into_raw_parts_with_allocator();
    assert!(actual_allocator.same_owner(&allocator));
    let memory = prepared.seal();
    let old = Layout::array::<u64>(capacity).map_err(|_| MemoryError::InvalidLayout)?;
    let smaller = Layout::array::<u64>(length).map_err(|_| MemoryError::InvalidLayout)?;
    let live = NonNull::new(pointer)
        .ok_or(MemoryError::InvalidLayout)?
        .cast::<u8>();
    // SAFETY: live is the still-owned vector allocation in this allocator,
    // old is its capacity layout and smaller fits all initialized elements.
    if let Ok(shrunk) = unsafe { actual_allocator.shrink(live, old, smaller) } {
        // SAFETY: successful shrink invalidated pointer. The returned block
        // contains all original initialized elements at the requested layout.
        drop(unsafe {
            Vec::from_raw_parts_in(
                shrunk.cast::<u64>().as_ptr(),
                length,
                length,
                actual_allocator,
            )
        });
        panic!("sealed allocation unexpectedly shrank");
    }
    // SAFETY: the pointer, initialized length, capacity and allocator came
    // from the same consumed Vec, and a refused shrink leaves them intact.
    let mut data = unsafe { Vec::from_raw_parts_in(pointer, length, capacity, actual_allocator) };
    assert!(data.try_reserve_exact(capacity).is_err());
    assert_eq!(data.as_ptr().addr(), address);
    assert_eq!(data.capacity(), capacity);
    assert_eq!(data.as_slice(), &[17, 29]);
    assert!(matches!(
        memory.allocator(REGION),
        Err(MemoryError::PreparedOnly)
    ));
    drop(memory);
    drop(allocator);
    data.push(41);
    assert_eq!(data.as_slice(), &[17, 29, 41]);
    Ok(())
}

#[test]
fn failed_bundle_rolls_back_and_control_survives_work_close() -> Result<(), MemoryError> {
    let memory = arena(4096, MemoryPolicy::PreparedReuse)?;
    let admission = Admission::prepare(
        &memory,
        REGION,
        [
            ResourceCapacity {
                unit: CapacityUnit::Slots,
                work: 2,
                control: 1,
            },
            ResourceCapacity {
                unit: CapacityUnit::Bytes,
                work: 8,
                control: 2,
            },
        ],
    )?;
    let initial = admission.usage();
    assert!(matches!(
        admission.reserve([2, 9], [0, 0]),
        Err(MemoryError::CapacityExceeded { resource: 1, .. })
    ));
    let refused = admission.usage();
    assert_eq!(refused.available_work, initial.available_work);
    assert_eq!(refused.high_water_work, initial.high_water_work);
    assert_eq!(refused.rejected_work, 1);
    assert!(matches!(
        admission.reserve_backed([1, 1], [0, 0], || Err::<(), _>(MemoryError::NoSlot)),
        Err(MemoryError::NoSlot)
    ));
    assert_eq!(admission.usage().available_work, [2, 8]);
    let held = admission.reserve([2, 8], [0, 0])?;
    admission.close();
    assert!(matches!(
        admission.reserve([1, 1], [0, 0]),
        Err(MemoryError::Closed)
    ));
    let control = admission.reserve_control([1, 2])?;
    assert_eq!(admission.usage().available_work, [0, 0]);
    assert_eq!(admission.usage().available_control, [0, 0]);
    drop(held);
    drop(control);
    assert_eq!(admission.usage().available_work, [2, 8]);
    assert_eq!(admission.usage().available_control, [1, 2]);
    Ok(())
}

#[test]
fn concurrent_last_alias_returns_once_and_closed_pool_retains_live_value()
-> Result<(), Box<dyn std::error::Error>> {
    let memory = arena(4096, MemoryPolicy::PreparedReuse)?;
    let pool = memory.slots(REGION, 1, |_| Ok(17_u64))?;
    let first = pool.try_checkout()?.share();
    let generation = first.id();
    let second = first.try_clone()?;
    let barrier = Barrier::new(3);
    std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            barrier.wait();
            assert_eq!(*first, 17);
            drop(first);
        });
        let right = scope.spawn(|| {
            barrier.wait();
            assert_eq!(*second, 17);
            drop(second);
        });
        assert_eq!(pool.active(), 1);
        assert!(matches!(pool.try_checkout(), Err(MemoryError::NoSlot)));
        barrier.wait();
        assert!(left.join().is_ok());
        assert!(right.join().is_ok());
    });
    let returned = pool.try_checkout()?;
    assert_ne!(returned.id(), generation);
    let held = returned.share();
    pool.close();
    drop(memory);
    assert!(matches!(pool.try_checkout(), Err(MemoryError::Closed)));
    assert_eq!(*held, 17);
    drop(held);
    assert_eq!(pool.active(), 0);
    Ok(())
}

struct PinnedProbe {
    address: Cell<Option<usize>>,
    _pinned: PhantomPinned,
}
impl Future for PinnedProbe {
    type Output = ();
    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<()> {
        let this = self.as_ref().get_ref();
        let address = std::ptr::from_ref(this).addr();
        match this.address.replace(Some(address)) {
            None => {
                ctx.waker().wake_by_ref();
                Poll::Pending
            }
            Some(previous) => {
                assert_eq!(previous, address);
                Poll::Ready(())
            }
        }
    }
}

#[test]
fn stable_frames_and_reuse_are_allocation_free_on_this_thread() -> Result<(), MemoryError> {
    let prepared = arena(4096, MemoryPolicy::PreparedReuse)?;
    let frames = prepared.frames(REGION, 1, Layout::new::<PinnedProbe>())?;
    let bytes = prepared.buffers(REGION, 1, Bytes::new(8))?;
    let memory = prepared.seal();
    let before = memory.usage(REGION)?.active_backing;
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let done = (|| {
        for _ in 0..128 {
            let mut lease = bytes.try_checkout()?;
            lease.clear();
            lease.extend_from_slice(b"bounded")?;
            let shared = lease.share();
            let retained = shared.try_clone()?;
            drop(shared);
            assert_eq!(retained.as_slice(), b"bounded");
            drop(retained);
            let mut frame = frames.reserve()?.initialize(PinnedProbe {
                address: Cell::new(None),
                _pinned: PhantomPinned,
            });
            let mut context = Context::from_waker(Waker::noop());
            assert!(Pin::new(&mut frame).poll(&mut context).is_pending());
            let mut moved_handle = frame;
            assert!(Pin::new(&mut moved_handle).poll(&mut context).is_ready());
        }
        Ok::<_, MemoryError>(())
    })();
    let allocations = ALLOCATIONS.with(|count| count.replace(None));
    done?;
    assert_eq!(allocations, Some(0));
    assert_eq!(memory.usage(REGION)?.active_backing, before);
    Ok(())
}

#[test]
fn guards_cover_error_unwind_and_never_polled_ownership() {
    let cleanup = Cell::new(0);
    let error = || -> io::Result<()> {
        let _guard =
            effect_drop_guard! { value: 2, cleanup: |units| cleanup.set(cleanup.get() + units) };
        Err(io::Error::other("early error"))?;
        Ok(())
    };
    assert!(error().is_err());
    assert_eq!(cleanup.get(), 2);
    let dismissed =
        effect_drop_guard! { value: 8, cleanup: |units| cleanup.set(cleanup.get() + units) };
    assert_eq!(std::mem::DropGuard::dismiss(dismissed), 8);
    assert_eq!(cleanup.get(), 2);
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _guard =
            effect_drop_guard! { value: 3, cleanup: |units| cleanup.set(cleanup.get() + units) };
        panic!("guard unwind");
    }));
    assert!(panic.is_err());
    let external =
        effect_drop_guard! { value: 5, cleanup: |units| cleanup.set(cleanup.get() + units) };
    let future = async move {
        let _guard = external;
        pending::<()>().await
    };
    drop(future);
    let counter = &cleanup;
    let future = async move {
        let _inside =
            effect_drop_guard! { value: 11, cleanup: |units| counter.set(counter.get() + units) };
        pending::<()>().await;
    };
    drop(future);
    assert_eq!(cleanup.get(), 10);
}

const SHRINK_CHILD: &str = "EFFECT_PREPARED_SHRINK_CHILD";
const SHRINK_MARKER: &str = "starting infallible shrink of sealed vector";

#[test]
#[ignore = "invoked as a dedicated allocation-abort child"]
fn infallible_shrink_child() -> Result<(), MemoryError> {
    if std::env::var_os(SHRINK_CHILD).is_none() {
        return Ok(());
    }
    let memory = arena(4096, MemoryPolicy::PreparedReuse)?;
    let mut data = memory.allocator(REGION)?.vector::<u64>(4)?;
    data.extend([17, 29]);
    let _sealed = memory.seal();
    eprintln!("{SHRINK_MARKER}");
    data.shrink_to_fit();
    Ok(())
}

#[test]
fn infallible_shrink_abort_is_isolated() -> io::Result<()> {
    let child = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--ignored",
            "--exact",
            "infallible_shrink_child",
            "--nocapture",
        ])
        .env(SHRINK_CHILD, "1")
        .output()?;
    let diagnostics = String::from_utf8_lossy(&child.stderr);
    assert!(!child.status.success());
    assert!(diagnostics.contains(SHRINK_MARKER), "{diagnostics}");
    assert!(
        diagnostics.contains("memory allocation of 16 bytes failed"),
        "{diagnostics}"
    );
    Ok(())
}
