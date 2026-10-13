//! Append two customer receipts through one reusable, scope-owned receipt-book file.
//!
//! Run: `cargo +1.99.0 run -p effect --example write_receipt_book -- OUTPUT.csv`.
//! Input: Maya's 950-cent receipt and Lee's 325-cent receipt. Output is those two
//! CSV rows. One pooled file stays open between appends; one permit per outlet
//! prevents overlapping writes, and scope close flushes and closes the file.
pub mod cafe_support;
use cafe_support::Cafe;
use effect::{
    Effect, Exit, PoolCapacity, Scope, effect_permit, effect_scope,
    effect_try_blocking, pool,
};
use std::num::NonZeroUsize;
use std::{fs::File, io::Write, path::PathBuf, sync::Arc};

/// File acquisition, append and final flush belong to the blocking provider selected by R.
fn write_book(path: Arc<PathBuf>) -> Effect<(), BookError, Cafe> {
    effect_scope! {
        body |scope: Scope<BookError>| {
            let path = path.clone();
            let files = pool! {
                scope: scope.clone(), capacity: PoolCapacity::Bounded(NonZeroUsize::MIN), idle_ttl: None,
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
                exists_idle; pool: files, predicate |file: &File| { file.metadata().is_ok() },
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
}
/// The caller names a fresh book; the run returns only after its pooled file is closed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = Arc::new(PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("provide a new receipt book")?,
    ));
    let env: Arc<Cafe> = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(write_book(path.clone()).run(env.clone()));
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
