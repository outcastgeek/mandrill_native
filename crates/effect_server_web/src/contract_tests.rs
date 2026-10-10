//! Fixed V06 Web contract cases. Core cache fencing and rate-store semantics are
//! credited to their existing suites; these cases cover Web policy/owner adaptation.

use crate::{
    WebError,
    admission::{AdmissionClass, Maintenance, MaintenanceMode, RetryAfter, SessionAdmission},
    availability::{
        Availability, AvailabilityError, Probe, ProbeDeadlines, ProbeRole, SignalState,
    },
    csp::{CspDirective, CspSource, CspSources, ResponseNonce},
    localization::{
        CatalogDefinition, CatalogLimits, CatalogResource, CatalogSelection, Contract, CultureId,
        CultureSet, Localization, LocalizationError, Message, MessageContract, MissingMessage,
        NamedArgument,
    },
    output_cache::{
        FiniteOutputPolicy, FiniteSource, OutputCache, OutputCacheDefinition, OutputPartition,
        RootOutputCache,
    },
    security::{ForwardedHeader, Security, TrustedForwarding},
};
use effect::{
    AdmissionCapacity, Cancellation, Effect, Exit, HasClock, HasTaskSpawner, Scope, SharedValue,
};
use effect_server::{HasRequest, Request};
use effect_tokio::{TokioClock, TokioTasks};
use std::{
    num::{NonZeroU64, NonZeroUsize},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Two workers are the existing TokioTasks adapter's minimum parallel provider.
const FIXTURE_WORKERS: usize = 2;
/// The availability case admits two independently scheduled probe workers.
const FIXTURE_PROBE_PARALLELISM: usize = 2;
/// Database, acknowledged and waiting are the three selected readiness signals.
const FIXTURE_READINESS_SIGNALS: usize = 3;
/// Each core timeout owns two race contenders. Reserve all three signal pairs,
/// the collection pair and the two workers, including contender release overlap.
const FIXTURE_TASK_CAPACITY: usize =
    2 * (FIXTURE_READINESS_SIGNALS + 1) + FIXTURE_PROBE_PARALLELISM;
/// Selected runtime/request services for finite Web checks; no hidden runtime.
struct Environment {
    /// The caller's monotonic service, shared by all fixture Effects.
    clock: TokioClock,
    /// Managed tasks include timeout contenders as well as bounded probe workers.
    tasks: TokioTasks,
    /// One native request/body owner used by output-cache checks.
    request: Request,
}
impl HasClock for Environment {
    /// Existing native clock selected through the fixture's sole R.
    type Clock = TokioClock;
    /// Borrow the retained service rather than creating a new time origin.
    fn clock(&self) -> &Self::Clock {
        &self.clock
    }
}
impl HasTaskSpawner for Environment {
    /// Existing native task provider, independent of the probe concurrency bound.
    type Tasks = TokioTasks;
    /// Borrow the caller-owned executor used by managed children.
    fn task_spawner(&self) -> &Self::Tasks {
        &self.tasks
    }
}
impl HasRequest for Environment {
    /// Keep the same metadata/body owner through each finite check.
    fn request(&self) -> &Request {
        &self.request
    }
}
/// Keep the caller-built reactor alive until all scope and provider cleanup settles.
fn environment()
-> Result<(Arc<tokio::runtime::Runtime>, Arc<Environment>), Box<dyn std::error::Error>> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(FIXTURE_WORKERS)
            .enable_all()
            .build()?,
    );
    // Native task admission includes timeout contenders, not only Tokio threads.
    let tasks = TokioTasks::new(
        runtime.clone(),
        AdmissionCapacity::Bounded(positive(FIXTURE_TASK_CAPACITY)?),
    )
    .map_err(|defect| std::io::Error::other(defect.message().to_owned()))?;
    let request = Request::from_native(
        http::Request::new(axum::body::Body::empty()),
        Cancellation::new(),
    );
    Ok((
        runtime.clone(),
        Arc::new(Environment {
            clock: TokioClock::new(runtime),
            tasks,
            request,
        }),
    ))
}
/// Literal positive bounds are scenario inputs; no unwrap/expect is needed.
fn positive(value: usize) -> Result<NonZeroUsize, Box<dyn std::error::Error>> {
    NonZeroUsize::new(value).ok_or_else(|| "positive fixture bound".into())
}

/// CSP's nonce is shared with rendering; none remains exclusive, and outer wrapping
/// preserves repeated fields on a finite short-circuit header owner.
#[test]
fn csp_nonce_and_outer_headers() -> Result<(), Box<dyn std::error::Error>> {
    let nonce = ResponseNonce::new("Y2FmZS1yZXNwb25zZQ==")?;
    let csp = crate::csp! { directive(CspDirective::ScriptSrc, CspSources::nonce()); directive(CspDirective::ObjectSrc, CspSources::none()); }?;
    assert!(csp.header(Some(&nonce))?.to_str()?.contains(nonce.as_str()));
    assert!(csp.header(None).is_err());
    assert!(CspSources::new(vec![CspSource::None, CspSource::SelfOrigin]).is_err());
    let mut selected = Security::<WebError, ()>::new();
    selected.csp = Some(csp);
    selected
        .headers
        .append("x-cafe", http::HeaderValue::from_static("first"));
    selected
        .headers
        .append("x-cafe", http::HeaderValue::from_static("second"));
    let mut headers = http::HeaderMap::new();
    selected.wrap_headers(&mut headers, Some(&nonce), false)?;
    assert_eq!(headers.get_all("x-cafe").iter().count(), 2);
    assert!(headers.contains_key("content-security-policy"));
    Ok(())
}
/// Forged left-prefix forwarding cannot override the first untrusted client; an
/// untrusted native socket ignores forwarding entirely.
#[test]
fn trusted_forwarding_suffix() -> Result<(), Box<dyn std::error::Error>> {
    let proxy = "127.0.0.1".parse()?;
    let policy = TrustedForwarding::new(vec![proxy], positive(3)?, ForwardedHeader::Standard)?;
    let mut headers = http::HeaderMap::new();
    headers.insert(
        "forwarded",
        http::HeaderValue::from_static(
            "for=192.0.2.9, for=198.51.100.4;proto=https;host=cafe.example",
        ),
    );
    let result = policy.resolve("127.0.0.1:8080".parse()?, &headers)?;
    assert_eq!(result.client.to_string(), "198.51.100.4");
    assert_eq!(result.scheme.as_deref(), Some("https"));
    let ignored = policy.resolve("203.0.113.7:8080".parse()?, &headers)?;
    assert_eq!(ignored.client.to_string(), "203.0.113.7");
    assert!(ignored.scheme.is_none());
    Ok(())
}
/// Local read-only policy admits reads but rejects writes, while session admission
/// remains held until the actual owning scope joins its finalizer.
#[test]
fn local_admission_and_scope_permit() -> Result<(), Box<dyn std::error::Error>> {
    let control = Maintenance::new(
        MaintenanceMode::ReadOnly,
        RetryAfter::seconds(NonZeroU64::MIN),
    );
    assert!(control.decision(AdmissionClass::Read).is_ok());
    assert!(control.decision(AdmissionClass::Mutation).is_err());
    let (runtime, env) = environment()?;
    let admission = SessionAdmission::new(NonZeroUsize::MIN);
    let selected = admission.clone();
    let owned = Scope::scoped(move |scope| {
        let owner = selected.clone();
        selected
            .acquire(scope, |_| WebError::Rejected("capacity"))
            .map(move |()| assert_eq!(owner.active(), 1))
    });
    assert!(matches!(
        runtime.block_on(owned.run(env.clone())),
        Exit::Success(())
    ));
    assert_eq!(admission.active(), 0);
    runtime.block_on(env.tasks.stop().wait());
    Ok(())
}
/// One typed catalogue identity and argument contract, with no generic stringifier.
struct Messages;
/// Greeting contract belongs only to the fixture catalogue family.
struct Greeting;
impl MessageContract for Greeting {
    /// Prevent cross-catalogue lookup.
    type Catalogue = Messages;
    /// Move one named customer value into a bound message.
    type Arguments = NamedArgument<String, ()>;
    /// Stable family identity used by resource validation.
    const CATALOGUE: &'static str = "cafe";
    /// Flattened resource key for the pickup greeting.
    const KEY: &'static str = "ready";
    /// Exact argument order accepted by the template.
    const FIELDS: &'static [&'static str] = &["customer"];
}
/// Valid reload updates a new generation; malformed replacement preserves the
/// previous generation and an already pinned request keeps its original text.
#[test]
fn catalog_reload_failure_and_request_pin() -> Result<(), Box<dyn std::error::Error>> {
    let (runtime, env) = environment()?;
    let version = Arc::new(AtomicUsize::new(0));
    let source = version.clone();
    let culture = CultureId::new("en-CA")?;
    let selected = culture.clone();
    let limits = CatalogLimits {
        bytes: positive(512)?,
        depth: positive(4)?,
        keys: positive(8)?,
        key_bytes: positive(64)?,
        value_bytes: positive(128)?,
        message_bytes: positive(128)?,
    };
    let resource = CatalogResource {
        identity: Arc::from("menu"),
        culture: culture.clone(),
        load: Arc::new(move || {
            let source = source.clone();
            Effect::sync(move || {
                bytes::Bytes::from_static(match source.load(Ordering::Acquire) {
                    0 => br#"{"ready":"Ready for {customer}"}"#,
                    1 => br#"{"ready":"Updated for {customer}"}"#,
                    _ => br#"{"ready":"{unknown}"}"#,
                })
            })
        }),
    };
    let definition = CatalogDefinition::<Messages, Environment>::new(
        "cafe",
        limits,
        vec![resource],
        vec![Contract::of::<Greeting>()],
    )?;
    let cultures = CultureSet::new(vec![culture.clone()], Vec::new(), culture)?;
    let localization = Localization::new(
        definition,
        cultures,
        move |_| {
            let selected = selected.clone();
            Effect::sync(move || CatalogSelection {
                ui: selected.clone(),
                formatting: selected.clone(),
                vary: Arc::from([]),
            })
        },
        MissingMessage::Fail,
        None,
    )?;
    let request = env.request.clone();
    let program = Scope::scoped(move |scope| {
        let version = version.clone();
        let request = request.clone();
        localization.scoped(scope, |error| error).flat_map(move |catalog| {
            let version = version.clone(); let request = request.clone();
            Effect::from_exit_future_with_context(move |environment: Arc<Environment>, context| {
                let catalog = catalog.clone(); let version = version.clone(); let request = request.clone();
                async move {
                    let Exit::Success(pinned) = context.run(catalog.for_request(request.clone()), environment.clone()).await else { return Exit::Fail(LocalizationError::Admission); };
                    let message = match crate::message_arguments! { message: Message::<Greeting>::new(), field("customer", String::from("Maya")); } { Ok(message) => message, Err(error) => return Exit::Fail(error) };
                    assert_eq!(pinned.text(&message).as_deref(), Ok("Ready for Maya"));
                    version.store(1, Ordering::Release);
                    assert!(matches!(context.run(catalog.reload(), environment.clone()).await, Exit::Success(())));
                    version.store(2, Ordering::Release);
                    assert!(matches!(context.run(catalog.reload(), environment.clone()).await, Exit::Fail(_)));
                    let Exit::Success(status) = context.run(catalog.status(), environment.clone()).await else { return Exit::Fail(LocalizationError::Admission); };
                    assert_eq!(status.generation, 1); assert!(status.failure.is_some());
                    assert_eq!(pinned.text(&message).as_deref(), Ok("Ready for Maya"));
                    let Exit::Success(current) = context.run(catalog.for_request(request), environment).await else { return Exit::Fail(LocalizationError::Admission); };
                    assert_eq!(current.text(&message).as_deref(), Ok("Updated for Maya")); Exit::Success(())
                }
            })
        })
    });
    assert!(matches!(
        runtime.block_on(program.run(env.clone())),
        Exit::Success(())
    ));
    runtime.block_on(env.tasks.stop().wait());
    Ok(())
}
/// A cached finite response still executes its authorization/precondition gate on
/// every access. Explicit partition variation belongs to the same bounded key.
#[test]
fn finite_output_gate_before_retained_hit() -> Result<(), Box<dyn std::error::Error>> {
    let (runtime, env) = environment()?;
    let gates = Arc::new(AtomicUsize::new(0));
    let renders = Arc::new(AtomicUsize::new(0));
    let audience = Arc::new(AtomicUsize::new(0));
    let policy = FiniteOutputPolicy {
        name: Arc::from("public-menu"),
        response_bytes: positive(256)?,
        key_bytes: positive(256)?,
        entries: positive(2)?,
        maximum_active: NonZeroUsize::MIN,
        retain: Duration::from_secs(1),
        vary_headers: Arc::from([]),
        vary_query: false,
    };
    env.request.insert_extension(RootOutputCache {
        definition: OutputCacheDefinition::new(vec![policy.clone()])?,
    });
    let counted = renders.clone();
    let source = FiniteSource::text(http::HeaderMap::new(), move |_| {
        let counted = counted.clone();
        Effect::<String, WebError, Environment>::sync(move || {
            counted.fetch_add(1, Ordering::AcqRel);
            String::from("Espresso")
        })
    })?;
    let counted = gates.clone();
    let program = Scope::scoped(move |scope| {
        let counted = counted.clone();
        let partition = audience.clone();
        let next_audience = audience.clone();
        OutputCache::scoped(
            scope,
            policy.clone(),
            source.clone(),
            move |_| {
                let counted = counted.clone();
                Effect::sync(move || {
                    counted.fetch_add(1, Ordering::AcqRel);
                })
            },
            |_| false,
            move |_, _| {
                Ok(OutputPartition {
                    namespace: Arc::from("cafe"),
                    dimensions: vec![
                        Arc::from(partition.load(Ordering::Acquire).to_string()),
                        Arc::from("en-ca"),
                        Arc::from("text"),
                    ]
                    .into(),
                })
            },
            |error| error,
        )
        .flat_map(move |cache| {
            let second = cache.clone();
            let third = cache.clone();
            let next_audience = next_audience.clone();
            cache.reply().flat_map(move |_| {
                let third = third.clone();
                let next_audience = next_audience.clone();
                second.reply().flat_map(move |_| {
                    next_audience.store(1, Ordering::Release);
                    third.reply().map(|_| ())
                })
            })
        })
    });
    let outcome: Exit<(), SharedValue<WebError>> = runtime.block_on(program.run(env.clone()));
    assert!(matches!(outcome, Exit::Success(())));
    assert_eq!(gates.load(Ordering::Acquire), 3);
    assert_eq!(renders.load(Ordering::Acquire), 2);
    runtime.block_on(env.tasks.stop().wait());
    Ok(())
}
/// Required readiness failure rejects traffic; optional health failure degrades;
/// liveness remains separately classified and completed signals survive timeout.
#[test]
fn distinct_availability_and_deadline_results() -> Result<(), Box<dyn std::error::Error>> {
    let (runtime, env) = environment()?;
    let probes = vec![
        Probe::new(
            "database",
            ProbeRole::Readiness,
            true,
            Effect::succeed(false),
        )?,
        Probe::new(
            "optional-report",
            ProbeRole::Health,
            false,
            Effect::succeed(false),
        )?,
        Probe::new("loop", ProbeRole::Liveness, true, Effect::succeed(true))?,
        Probe::new(
            "acknowledged",
            ProbeRole::Readiness,
            false,
            Effect::succeed(true),
        )?,
        Probe::new(
            "waiting",
            ProbeRole::Readiness,
            false,
            // Native futures retain ownership until cooperative cancellation
            // settles; the timeout then joins this loser before reporting expiry.
            Effect::from_future_with_env(|_: Arc<Environment>, cancellation| async move {
                cancellation.cancelled().await;
                false
            }),
        )?,
    ];
    let availability = Availability::new(
        "cafe",
        |error| error,
        probes,
        ProbeDeadlines {
            per_probe: Duration::from_millis(50),
            overall: Duration::from_secs(1),
        },
        positive(FIXTURE_PROBE_PARALLELISM)?,
        None,
    )?;
    let ready: Exit<_, AvailabilityError> =
        runtime.block_on(availability.collect(ProbeRole::Readiness).run(env.clone()));
    let Exit::Success(ready) = ready else {
        return Err(match ready {
            Exit::Die(defect) => defect.message().to_owned(),
            _ => String::from("readiness did not settle"),
        }
        .into());
    };
    assert!(!ready.accepting_traffic);
    assert!(
        ready
            .signals
            .iter()
            .any(|signal| signal.name.as_ref() == "acknowledged"
                && signal.state == SignalState::Available)
    );
    assert!(
        ready.signals.iter().any(
            |signal| signal.name.as_ref() == "waiting" && signal.state == SignalState::TimedOut
        )
    );
    let Exit::Success(health) =
        runtime.block_on(availability.collect(ProbeRole::Health).run(env.clone()))
    else {
        return Err("health did not settle".into());
    };
    assert!(health.accepting_traffic && health.degraded);
    let Exit::Success(live) =
        runtime.block_on(availability.collect(ProbeRole::Liveness).run(env.clone()))
    else {
        return Err("liveness did not settle".into());
    };
    assert!(live.accepting_traffic && !live.degraded);
    runtime.block_on(env.tasks.stop().wait());
    Ok(())
}
