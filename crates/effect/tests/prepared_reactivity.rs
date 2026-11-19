//! Prepared graph mutations publish only after admitting their entire invalidation.

use effect::memory::{Bytes, MemoryError, RegionId};
use effect::reactivity::{
    AtomId, AtomKind, PreparedAtomRegistry, PreparedAtomRegistryCapacity,
    PreparedAtomRegistryError, PreparedInvalidationCapacity, PreparedInvalidationError,
    PreparedInvalidationRegistry,
};
use effect::{
    CallbackOverflow, KeyDomain, KeyNamespace, KeySegment, PanicMessageOverflow,
    PreparedPanicConfig, PreparedPanicPool, PreparedPubSubCapacity, ReplayPolicy, StableKey,
    memory,
};
use std::fmt::Debug;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

const WORK: RegionId = RegionId::named("work");
const ONE: NonZeroUsize = NonZeroUsize::MIN;
const TWO: NonZeroUsize = ONE.saturating_add(1);

struct TestDomain;

impl KeyDomain for TestDomain {
    const TAG: &'static str = "prepared-atom-test";
}

fn checked<T, E: Debug>(result: Result<T, E>) -> Result<T, String> {
    result.map_err(|error| format!("{error:?}"))
}

#[test]
fn graph_fanout_refusal_preserves_edges_versions_and_evaluation() -> Result<(), String> {
    let plan = checked(memory! {
        total: 128 KiB, mode: prepared, metadata: work, partition work: 128 KiB,
    })?;
    let prepared = checked(plan.prepare())?;
    let panics = checked(PreparedPanicPool::prepare(
        &prepared,
        WORK,
        PreparedPanicConfig {
            slots: 1,
            bytes_per_message: Bytes::new(64),
            overflow: PanicMessageOverflow::Utf8Prefix,
        },
    ))?;
    let invalidations = Arc::new(checked(PreparedInvalidationRegistry::prepare(
        &prepared,
        WORK,
        PreparedInvalidationCapacity {
            keys: TWO,
            keys_per_batch: ONE,
            bytes_per_key: Bytes::new(128),
            delivery: PreparedPubSubCapacity {
                payloads: 1,
                publishers: 1,
                subscribers: 1,
                takers_per_subscriber: 1,
            },
            buffer: ONE,
            overflow: CallbackOverflow::DropNewest,
            replay: ReplayPolicy::None,
        },
        &panics,
    ))?);
    let graph = checked(PreparedAtomRegistry::prepare(
        &prepared,
        WORK,
        PreparedAtomRegistryCapacity {
            atoms: TWO,
            edges: ONE,
            dependencies: ONE,
            operations: ONE,
            evaluations: ONE,
            snapshots: ONE,
            bytes_per_identity: Bytes::new(128),
        },
        Arc::clone(&invalidations),
    ))?;
    let _sealed = prepared.seal();
    let namespace = checked(KeyNamespace::<TestDomain>::new("graph"))?;
    let source_id = AtomId::from_stable(checked(StableKey::new(
        &namespace,
        [KeySegment::Unsigned(1)],
    ))?);
    let derived_id = AtomId::from_stable(checked(StableKey::new(
        &namespace,
        [KeySegment::Unsigned(2)],
    ))?);
    let source = checked(graph.register(source_id.clone(), AtomKind::Source))?;
    let derived = checked(graph.register(derived_id.clone(), AtomKind::Derived))?;
    checked(graph.replace_dependencies(&derived, [source_id.clone()]))?;
    let retained = checked(graph.dependencies_of(&derived))?;
    let mut evaluation = checked(graph.begin_evaluation(&derived))?;
    checked(graph.observe(&mut evaluation, &source))?;

    assert!(matches!(
        graph.changed(&source),
        Err(PreparedAtomRegistryError::Invalidation(
            PreparedInvalidationError::Memory(MemoryError::CollectionReservationFailed)
        ))
    ));
    let source_key = source_id.invalidation_key();
    let derived_key = derived_id.invalidation_key();
    assert_eq!(
        invalidations.revision_of(&[source_key.clone(), derived_key.clone()]),
        None
    );
    assert!(graph.is_current(&evaluation));
    assert!(checked(graph.commit_evaluation(evaluation))?);
    assert_eq!(retained.as_slice(), &[source_id]);
    assert!(matches!(
        graph.dependencies_of(&derived),
        Err(PreparedAtomRegistryError::Memory(MemoryError::NoSlot))
    ));

    checked(graph.replace_dependencies(&derived, []))?;
    checked(graph.changed(&source))?;
    assert_eq!(
        invalidations.revision_of(&[source_key]),
        Some(NonZeroU64::MIN.saturating_add(1))
    );
    assert_eq!(
        invalidations.revision_of(&[derived_key]),
        Some(NonZeroU64::MIN)
    );
    assert_eq!(retained.as_slice().len(), 1);
    drop(retained);
    assert!(
        checked(graph.dependencies_of(&derived))?
            .as_slice()
            .is_empty()
    );
    Ok(())
}
