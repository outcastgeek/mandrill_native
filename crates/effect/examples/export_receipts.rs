//! Export per-customer receipt totals from a caller-supplied receipt CSV.
//!
//! Run: `cargo +1.99.0 run -p effect --example export_receipts -- COUNTER.csv MOBILE.csv OUTPUT.csv`.
//! Each input file contains rows of `ticket,customer,cups,unit_cents`, with no header or quoted commas.
//! For `104,Maya,2,475` and `105,Lee,1,325`, output is `Lee,325` then `Maya,950`.
//! Two bounded workers read independent counter/mobile files and validate their rows.
//! CPU workers aggregate customer
//! totals; owned batches move between steps. The output must not already exist.

use effect::effect_try_sync;
pub mod cafe_support;
use cafe_support::Cafe;
use effect::{
    Effect, Exit, effect, effect_blocking, effect_cpu, effect_for_each_concurrent,
    effect_try_blocking, effect_try_cpu,
};
use std::num::NonZeroUsize;
use std::{collections::BTreeMap, io::Write, path::PathBuf, sync::Arc};

/// Read a real batch, validate its records, calculate customer totals and write the export.
fn export(inputs: Arc<[Arc<PathBuf>]>, output: PathBuf) -> Effect<usize, ExportError, Cafe> {
    effect! {
        let inputs = inputs.clone(); let output = output.clone();
        let snapshots = effect_for_each_concurrent! {
            concurrency: NonZeroUsize::MIN.saturating_add(1), values: inputs.to_vec(),
            body |input: &Arc<PathBuf>| {
                let input = input.clone();
                let native = effect_blocking! {
                    once;
                    body |_cancel| { std::fs::read_to_string(input.as_ref()) },
                }?;
                let csv = effect_try_sync! { once; body: { let __dsl_result = native.map_err(ExportError::Io); move || __dsl_result }, on_error: ::core::convert::identity, }?;
                Arc::new(csv)
            },
        }?;
        let snapshots: Arc<[Arc<String>]> = snapshots.into();
        let batches = effect_for_each_concurrent! {
            shared; concurrency: NonZeroUsize::MIN.saturating_add(1), values: snapshots,
            body |csv: &Arc<String>| {
                let csv = csv.clone();
                effect_try_cpu! { once; body |_cancel| { read_receipts(&csv) }, }?
            },
        }?;
        let totals = effect_cpu! {
            once;
            body |_cancel| {
                let mut totals = BTreeMap::<String, u128>::new();
                for receipt in batches.into_iter().flatten() {
                    *totals.entry(receipt.customer).or_default() +=
                        u128::from(receipt.cups) * u128::from(receipt.unit_cents);
                }
                totals
            },
        }?;
        effect_try_blocking! {
            once;
            body |_cancel| {
                let mut file = std::fs::File::options().write(true).create_new(true)
                    .open(output).map_err(ExportError::Io)?;
                for (customer, cents) in &totals {
                    writeln!(file, "{customer},{cents}").map_err(ExportError::Io)?;
                }
                file.sync_all().map_err(ExportError::Io)?;
                Ok(totals.len())
            },
        }?
    }
}
/// Validated business row; no Clone is needed to transfer a batch to its next worker.
struct Receipt {
    /// Customer receiving the invoice total.
    customer: String,
    /// Strict positive cup count; this width also bounds total arithmetic.
    cups: u16,
    /// Nonnegative unit price in integer cents.
    unit_cents: u32,
}
/// File failures remain distinct from malformed receipt rows.
#[derive(Debug)]
pub enum ExportError {
    /// Original input/output file error.
    Io(std::io::Error),
    /// One-based line that does not match the declared receipt format.
    Row(#[doc = "Malformed receipt line."] usize),
}
/// Validate complete rows before any output file is created.
/// u16 cups and u32 cents summed over an addressable Vec fit u128 on this target.
fn read_receipts(csv: &str) -> Result<Vec<Receipt>, ExportError> {
    csv.lines()
        .enumerate()
        .map(|(index, line)| {
            let fields = line.split(',').collect::<Vec<_>>();
            let [number, customer, cups, unit_cents] = fields.as_slice() else {
                return Err(ExportError::Row(index + 1));
            };
            if number.is_empty() || customer.trim().is_empty() {
                return Err(ExportError::Row(index + 1));
            }
            let cups = cups
                .parse::<u16>()
                .ok()
                .filter(|cups| *cups > 0)
                .ok_or(ExportError::Row(index + 1))?;
            let unit_cents = unit_cents
                .parse::<u32>()
                .map_err(|_| ExportError::Row(index + 1))?;
            Ok(Receipt {
                customer: customer.trim().to_owned(),
                cups,
                unit_cents,
            })
        })
        .collect()
}
/// The caller selects the files; one R provides independent CPU and blocking execution.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut paths = std::env::args_os().skip(1).map(PathBuf::from);
    let counter = paths.next().ok_or("provide counter receipt CSV")?;
    let mobile = paths.next().ok_or("provide mobile receipt CSV")?;
    let output = paths.next().ok_or("provide new output CSV")?;
    assert!(
        paths.next().is_none(),
        "only COUNTER.csv MOBILE.csv OUTPUT.csv are accepted"
    );
    let inputs: Arc<[Arc<PathBuf>]> = [Arc::new(counter), Arc::new(mobile)].into();
    let env: Arc<Cafe> = Cafe::open(
        (),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let result = futures_executor::block_on(export(inputs, output.clone()).run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(customers) => {
            use std::io::Write;
            writeln!(
                std::io::stdout().lock(),
                "Exported {customers} customer totals to {}.",
                output.display()
            )?;
        }
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("receipt export failed: {other:?}").into());
        }
    }
    Ok(())
}
