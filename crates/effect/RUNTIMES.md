# Provider contracts and execution costs

The public type is `Effect<A, E, R>`. R contains application services and any task,
CPU, blocking, clock, transport, logging, and tracing providers together. A subtree
can project another R with a different provider while retaining the execution's
cancellation/resource hierarchy. There is no backend parameter or run-options
provider table. A Pool/Cache retains its construction environment through owned
closures so its resources and producers keep that environment alive.

| Provider | Supplied owner / configuration | Contract |
| --- | --- | --- |
| `TokioTasks` | Arc<Runtime>, explicit AdmissionCapacity | Multi-thread runtime with more than one actual worker; independent tasks, prompt admission, no inline user work; stop/drain joins retained drivers |
| `TokioBlocking` | Arc<Runtime>, capacity, Diagnostics | spawn_blocking owns jobs; cancellation does not abort a running callback; stop/drain waits settlement and worker diagnostics |
| `TokioClock` | Arc<Runtime> | Timer constructed under the supplied reactor; owned future retains the runtime; checked deadlines and cancellation |
| `RayonCpu` | Arc<ThreadPool>, capacity, Diagnostics | pool.spawn owns a synchronous job; no install/block_on fallback; stop/drain joins jobs and diagnostics |
| `NativeTasks` | Existing RuntimeHandle, Diagnostics | Native owner retains shutdown/terminal authority; typed admission mapping; bridge requests cancellation and awaits work |
| `TracingBackend` | tracing::Dispatch, positive active-span capacity, Diagnostics | Explicit dispatcher across threads, explicit parents, bounded span ownership, redaction and outcome recording |
| Custom provider | Consumer implements the public capability | ScheduledTask/ScheduledJob must remain owned through settlement; core detects accepted work loss |

The runtime/pool owner stays alive until application scopes, clock users,
protected releases, and provider drains settle. `ProviderAdmission` distinguishes
capacity slots from physically retained drivers. Capacity is released before
Effect completion is published; drain waits through publication/worker diagnostics.
This permits sequential work with capacity one without premature shutdown.

The existing native runtime owns its own admission permit through its done
callback. Its capacity therefore covers that full native operation lifetime;
Effect does not rewrite the workspace's native ownership protocol. Size native
admission for that lifetime. Native and Effect request tokens are linked without
calling another owner's private terminal/finish operation.

Admission rejects with typed ProviderCapacityFull/ProviderClosed defects. Nested
work does not block awaiting another permit on the worker that holds the current
one. Bounded traversal uses an explicit positive worker count; providers may have
additional independent admission bounds. A CPU callback must cooperate itself
when prompt cancellation matters. Arbitrary synchronous callbacks cannot be
preempted safely; accepted work is joined rather than detached.

## Managed transport obligations

`HttpTransport` and `WebSocketTransport` select no SDK or global client. The
consumer supplies a compatible native client/reactor through R. Their futures
own all native handles needed through settlement. An SDK returning a Rust Future
still has its own runtime requirements.

HTTP acquisition installs response ownership before a late cancellation check.
Status rejection and decoding failure still close the response. Response close
stops read admission, aborts outstanding native reads, joins them, then awaits
body disposal and retains cleanup defects. A body implementation must make abort
settle its admitted reads. Byte decoding distinguishes typed InvalidUtf8 from
HTTP status and transport failure.

WebSocket send and receive each serialize FIFO in their direction. Receive
assembles fragments within the caller's message bound and validates UTF-8 and
fragment sequencing. The provider honors the supplied receive-buffer request.
Close uses the R clock and explicit timeout; timeout/cancellation aborts and
joins the handshake. Release aborts, joins all admitted operations, and awaits
dispose. A provider that cannot make abort settle work cannot meet this contract.
A concrete network SDK adapter must supply these lifecycle guarantees; the
core supplies only the transport contract.

## Ownership and failure limits

Reusable descriptions and shared handles clone ownership handles, not A/E data.
Owned factories and continuations move payloads. Borrowed inspection cannot
escape its gate. Pool resources require Send + Sync; shared outcomes support
Send-but-not-Sync data through guarded inspection. Drop cannot await fallible
native release. Lost owners invalidate handles and report typed defects; this
is not a promise that abandoned asynchronous cleanup completed.

Abandoning a Pool/Cache scope synchronously freezes admission, requests producer
cancellation, and invalidates escaped pool leases. WeakCache also rejects work
through an existing consumer-held association after its core owner is lost.
Retained handles can explicitly run shutdown to join producers and dispose pool
resources; the acknowledgement reports owner loss. This recovery drain cannot
resurrect the abandoned scope's canonical outcome. No asynchronous callback runs
from Drop.

Scopes retain canonical cleanup E values; close/shutdown/lease acknowledgements
share status. Cache waiters share `SharedValue<A>` and `SharedValue<E>` because
Rust cannot duplicate a non-Clone outcome for multiple consumers. Weak-cache
successful associations use weak key and weak entry references and consumer
ownership, not ephemerons. Active producers and unobserved cleanup are separate,
strongly owned lifetimes.

## Execution costs

Recipe sharing avoids reconstructing descriptions; each run still creates
continuations and cancellation state and performs FlatDrop work, locking and
Arc ownership operations. Concurrent execution retains shared source/factory
state, result slots, worker join futures and per-item execution contexts. Fixed
workers avoid per-input task submission. These mechanisms incur runtime costs
whose impact depends on the application workload.

Macro parsing/lowering, Rust ownership/capability checks, static metadata, typed
constants, and empty annotation initialization execute at compilation. Dynamic
programs, provider admission, cancellation, JSON input, resource state, and live
schedule decisions execute at runtime.
