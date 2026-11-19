//! Append two customer receipts through one reusable, scope-owned receipt-book file.
//!
//! Run: `cargo +1.100.0 run -p effect --example write_receipt_book -- OUTPUT.csv`.
//! Input: Maya's 950-cent receipt and Lee's 325-cent receipt. Output is those two
//! CSV rows. One pooled file stays open between appends; one permit per outlet
//! prevents overlapping writes, and scope close flushes and closes the file.
pub mod cafe_support;
use cafe_support::Cafe;
use effect::memory::{Bytes, MemoryError, PreparedMemory, RegionId};
use effect::{
    Effect, Exit, Pool, PreparedPoolSnapshots, PreparedPoolStorage, PreparedSemaphoreError, Scope,
    effect_permit, effect_scope, effect_try_blocking, memory, pool,
};
use std::num::NonZeroUsize;
use std::{fs::File, io::Write, path::PathBuf, sync::Arc};

const FILES: NonZeroUsize = NonZeroUsize::MIN;
const WAITERS: usize = 1;
const POPULATION_CALLS: NonZeroUsize = NonZeroUsize::MIN;
// The scope retains one finalizer for each receipt's lease until close.
const LEASE_CONTROLS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(1);
const CLEANUP_OUTCOMES: NonZeroUsize = NonZeroUsize::MIN.saturating_add(2);
const CLEANUP_ENTRIES: usize = 4;
const BOOK_REGION: RegionId = RegionId::named("book");

/// The selected blocking provider acquires, writes and flushes the file.
fn write_book(
    path: Arc<PathBuf>,
    memory: Arc<PreparedMemory<'static>>,
    snapshots: Arc<PreparedPoolSnapshots<File, BookError>>,
) -> Effect<(), BookError, Cafe> {
    effect_scope! {
        body |scope: Scope<BookError>| {
            let path = path.clone();
            let memory = memory.clone();
            let snapshots = snapshots.clone();
            let files = pool! {
                scope: scope.clone(),
                memory: PreparedPoolStorage {
                    memory: memory.clone(), region: BOOK_REGION, capacity: FILES,
                    waiters: WAITERS, population_calls: POPULATION_CALLS,
                    lease_controls: LEASE_CONTROLS, cleanup_outcomes: CLEANUP_OUTCOMES,
                    cleanup_entries: CLEANUP_ENTRIES, on_error: Arc::new(BookError::PoolAdmission),
                },
                idle_ttl: None,
                create {
                    let path = path.clone();
                    effect_try_blocking! {
                        body |_cancel| { File::options().write(true).create_new(true).open(path.as_ref()).map_err(BookError::Io) },
                    }?
                },
                validate |file: &File| { let writable = file.metadata().is_ok(); writable },
                dispose |file: File| {
                    effect_try_blocking! { once; body |_cancel| { file.sync_all().map_err(BookError::Io) }, }?;
                },
            }?;
            files.ensure_size(FILES.get())?;
            let outlet = effect_permit! { partitioned_make; capacity: NonZeroUsize::MIN, };
            let using_files = files.clone(); let using_scope = scope.clone();
            effect_permit! {
                partitioned; semaphore: outlet, key: String::from("front-counter"), permits: NonZeroUsize::MIN,
                on_error: BookError::Admission,
                body {
                    let files = using_files.clone(); let scope = using_scope.clone();
                    for receipt in ["104,Maya,950", "105,Lee,325"] {
                        let file = files.acquire(scope.clone())?;
                        let writing = file.clone();
                        effect_try_blocking! {
                            once;
                            body |_cancel| {
                                writing.with(|file| { let mut file = file; writeln!(file, "{receipt}") })
                                    .map_err(BookError::Owner)?.map_err(BookError::Io)
                            },
                        }?;
                        file.release()?;
                    }
                },
            }?;
            let reusable = pool! {
                exists_idle; pool: files, memory: snapshots.clone(),
                on_capacity |error| { BookError::Inspection(error) },
                predicate |file: &File| { file.metadata().is_ok() },
            }?;
            assert!(reusable, "the pooled file must return after its append lease");
        },
    }
}
/// Book I/O, checked ownership and outlet admission retain their original causes.
#[derive(Debug)]
pub enum BookError {
    /// Native file acquisition, write or flush failed.
    Io(std::io::Error),
    /// A checked file owner was unavailable.
    Owner(effect::Defect),
    /// Per-outlet admission failed.
    Admission(effect::SemaphoreError),
    /// Pool control, resource or observer admission failed.
    PoolAdmission(PreparedSemaphoreError),
    /// The idle inspection could not reserve its snapshot.
    Inspection(MemoryError),
}
/// The caller names a fresh book; the run returns only after its pooled file is closed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = Arc::new(PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("provide a new receipt book")?,
    ));
    let book_bytes = Pool::<File, BookError>::prepared_storage_bytes(
        FILES,
        WAITERS,
        POPULATION_CALLS,
        LEASE_CONTROLS,
        CLEANUP_OUTCOMES,
        CLEANUP_ENTRIES,
    )?
    .checked_add(PreparedPoolSnapshots::<File, BookError>::storage_bytes(
        NonZeroUsize::MIN,
        FILES,
    )?)?
    .checked_add(Bytes::new(32 * 1024))?; // Region allocator headers and alignment.
    let metadata_bytes = Bytes::new(32 * 1024);
    let total = book_bytes
        .checked_add(metadata_bytes)?
        .checked_add(Bytes::new(4096))?;
    let memory = Arc::new(
        memory! {
            total: total, mode: prepared, metadata: metadata,
            partition metadata: metadata_bytes, partition book: book_bytes,
        }?
        .prepare()?,
    );
    let snapshots = Arc::new(PreparedPoolSnapshots::prepare(
        &memory,
        BOOK_REGION,
        NonZeroUsize::MIN,
        FILES,
    )?);
    let env: Arc<Cafe> = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result =
        futures_executor::block_on(write_book(path.clone(), memory, snapshots).run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => {
            use std::io::Write;
            writeln!(
                std::io::stdout().lock(),
                "Receipt book flushed and closed: {}.",
                path.display()
            )?;
        }
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("receipt book failed: {other:?}").into());
        }
    }
    Ok(())
}
