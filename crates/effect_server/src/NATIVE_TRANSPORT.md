# Prepared endpoint and transport status

`Router::prepared_methods` registers a prepared endpoint for a unit route.
`PreparedServer` provides explicit finite endpoint, connection, child, request,
terminal-origin, and cleanup storage. Its one-shot `with_host` returns a guarded
`PreparedHostResult` containing the complete host outcome. Successful settled
requests return their terminal records; unhandled outcomes retain origins until
that guarded result and its descendants retire. `prepared_input_methods` accepts
a typed `Route<A>` and a prepared endpoint that consumes `A` once. Admission
reserves the graph and input cell before the route decoder runs; selected shared
environment publication precedes the graph factory.

`PreparedServer::with_request_control_admission` optionally selects a caller's
single-resource `Slots` coordinator. Front admission claims one work unit only
after the actual request-control and other front owners are acquired; refusal
uses the existing prework 503 path. The request-control record and its aliases
retain that unit until their last loan retires. The same coordinator may be
shared with other slot owners. This accounts for the request-control slot, not
headers, transport backing, or application-owned values.

`PreparedServer::with_request_queue` optionally selects a caller-prepared
`PreparedSemaphore` and a positive wait deadline. The tracked HTTP service
acquires one permit in FIFO order before taking front-phase request leases,
running routing, or polling the incoming body. The parked service future
retains the parsed native head and `Incoming` handle. The semaphore's regional
storage covers its finite waiter and notification records. Account for the
native service future, parsed head, parser and protocol buffers, Tokio timer,
and inline host-stop listener in the transport envelope.

Queue saturation and expiry use the configured overload reply. Dropping the
service future on disconnect removes its wait entry. Host stop also cancels
the wait and leaves the caller's semaphore available to its other users.
After a grant the ordinary front-phase ownership checks still apply. Native
and graph response bodies and emitted DATA retain a producer loan until
they retire. The scheduled task retains its producer loan through request
cleanup. The last producer retires pending transport ownership and releases the
permit. A retained failure diagnostic can continue to own its control record
through scope settlement. Hosts that omit the queue keep immediate admission.

`PreparedServer::with_body_completion_admission` optionally charges one work
slot for each actual body completion record after its initial waiter is claimed.
The record's last body or wait owner retires the charge. Channel backing,
native bytes and network settlement are separate.
Each request and body control also owns an independently allocated startup Arc
for its loan gate. Demand reports the gate count and payload layout separately.

`PreparedServer::with_connection_admission` optionally charges one work slot
for an admitted connection record. The actual record's final shared owner
returns the slot before the charge is released; transport backing is separate.

`PreparedServer::with_observer_batch_admission` optionally charges one work
slot per admitted observer batch vector. Observer captures and their allocations
are separate from that charge.

`PreparedServer::with_terminal_admission` optionally charges one work slot per
terminal-origin record. Its final shared record owner retires the origin and
physical slot before releasing the charge.

`PreparedServer::with_request_storage` selects caller-prepared finite base-request
and metadata-snapshot records. Front admission reserves the base and three
snapshot vacancies before creating the ordinary cancellation child; selected
hosting requires at least four snapshot slots for one builtin routing flow.
`PreparedRequestStoragePool::with_admission` optionally charges one Slots work
unit for each retained root or snapshot; their independent loan gates and
startup Arc payloads are included in demand. HTTP containers and their native
or caller-owned payloads remain separate.
`PreparedRequestStorageCapacity::builtin_snapshots_for` checks four times a
chosen concurrent-flow count; hooks or callers retaining extra snapshots need
additional capacity. Subsequent selected routing/dispatch snapshots are admitted
before copying native parts or moving body ownership. `PreparedRequestStoragePool::demand` reports region
payload plus separate startup Arc control counts and payload bytes; Arc headers,
allocator rounding, native HeaderMap/Extensions/URI backing and cancellation
remain separate. `ResponseHeaders` still uses its ordinary per-request
`Arc<Mutex<HeaderMap>>`; its control, buckets, and values are not provided by
this request-storage pool. The ordinary Request and RoutingRequest methods remain available;
opaque legacy hooks that call those infallible methods are not certified prepared.

Prepared hosting requires explicitly provisioned return listeners on both the
cleanup buffer pool and its frame pool. Use
`PreparedCleanupPool::prepare_with_return_listeners` and
`PreparedMemory::frames_with_return_listeners`, selecting at least one listener
slot for each concurrently waiting host. Their corresponding
`storage_bytes_with_return_listeners` methods include the listener backing.
Before hosting begins, three cleanup buffers are retained for host/serving
control; startup also verifies two buffers for one connection when enabled,
and two for one request when request admission is enabled. The resulting
minimum is seven buffers and frames when both are enabled. Result frames and
any independently admitted graph cleanup needs remain separate actual claims,
including when pools are shared.

Prepared request-control records include the single outer routing owner,
inner fiber handle, outer body-signal state and native-slot request shadow; each inner response
record separately owns its body signal. Their storage is included in the respective region payloads; ordinary request metadata,
cancellation and routing scopes still have their own allocation obligations.
Native `Cancellation::cancelled` stores its listener inline in the returned
future; its owner's actual future layout must accommodate that storage. Lazy
native Event state, cancellation child links and propagation remain separate.
Cancellation propagation uses an intrusive work link in each native State,
rather than allocating traversal vectors; that increases each State's native
layout and does not prepare the State Arc, child vector or Event backing.
The routing value is taken before NativeSlot is inserted into the routed native
request, and an abandoned value retires before its control record can be reused.
The selected inner fiber handle uses that record's own storage and is taken for
the existing join; any abandoned handle is dropped after the last control loan,
outside its metadata gates. The fiber and native scope still have their own
ordinary execution allocations.

The selected outer native request fork retains its existing request-control
owner in both its scope cleanup and scheduled task. An accepted successful unit
task removes its own registered cleanup from the still-open host scope; pending
or non-successful tasks retain their cleanup and owner through normal scope
settlement. The task owner follows the scheduled future's captures in field
drop order, retaining the control until that future has been destroyed. This
bounds those retained selected registrations by actual request
controls, not a lifetime request count. The native Fiber, scope finalizer Vec,
task and cancellation backing remain separately owned native allocations.

At host startup the response-hook and selection-decorator catalogues become
two native `Arc` slices. Their element payloads and control/allocator overhead
are separate from prepared region demand; the callback owners and their
pointees are also separate. Requests borrow or clone these existing catalogue
owners rather than cloning their vectors. Other host hook catalogues remain
in the retained Boundary and are iterated by borrow.

Prepared endpoint dispatch selects its existing request output owner locally;
it does not allocate a new endpoint-body Arc for each request. The startup
registered endpoint body remains the ordinary endpoint path, and native
continuations and execution storage remain separate.

Each selected inner-response record contains two prepared reply-continuation
cells, included in its actual region demand. Strong handles retain the real
inner control and request owner; weak handles retain only a weak inner control
and a checked cell generation, so a retired cell cannot be resurrected through
an old weak handle. Last-owner retirement clears delivery data outside the
cell gate. Concurrent reentry beyond available cells refuses with a typed
memory error; ordinary Arc-backed continuations remain unchanged. Native
pipeline terminal closures remain separate. The private policy observer
catalogue uses a native startup Arc and Vec: policy clones share that owner,
while configuration of a shared policy copies the catalogue before mutation.
The Arc control, Vec capacity, observer callback owners and their pointees
are separate from prepared region demand.

Front request admission claims the first actual selected inner-response
record, its channel, an extra waiter on the preclaimed outer body, and two
reply-body vacancies before routing whenever that first inner reservation is
configured, independently of the watch-specific configuration check.
Cancellation is bound when the outer
body is published; the extra waiter is not polled early. The temporary inner
reservation does not hold the root request-control loan, so an unused
reservation can retire without a self-cycle. Dispatch publishes it for the
first inner response; further responses use the same finite inner pool and
claim their own waiter and vacancies. Retained strong or weak inner handles
or insufficient body capacity can therefore cause a prework refusal.

`PreparedResponseHeadersPool` optionally prepares a finite registry of the
existing public `ResponseHeaders` Arc controls. Select it with
`PreparedServer::with_response_headers_pool`; request admission claims and
clears one control before preparing request metadata. A retained strong or Weak
handle prevents that control's reuse. The registry intentionally owns a strong
Arc until the pool retires, so `Arc::get_mut`, `try_unwrap`, and Weak upgrade
observe that additional owner. Ordinary `ResponseHeaders::default()` keeps its
existing behavior. Demand separately reports region registry/index bytes,
startup control payloads, and the caller's pool Arc; standard Arc headers,
allocator rounding, mutable HeaderMap buckets and values are not bounded by
this pool.

```rust,ignore
let demand = PreparedResponseHeadersPool::demand(simultaneous_headers)?;
let headers = Arc::new(PreparedResponseHeadersPool::prepare(
    memory, region, simultaneous_headers,
)?);
let prepared = prepared.with_response_headers_pool(headers);
```

Temporary pre-accept cleanup saturation waits for actual buffer and frame
returns without retaining a partial connection or cleanup bundle. The duplicate
connection admission record returns after the execution, body and terminal origin
owners are installed; those actual owners still consume their own capacity. Listener
storage is prepared; external runtime waker implementations are not accounted
for by that listener storage.

For prepared HTTP/2 hosting, the caller-selected `requests_per_connection` also
sets Hyper's maximum concurrent peer-initiated stream count before serving.
Zero permits no new peer streams; counts that do not fit `u32` are rejected
before listening. Hyper can still allocate parser/header/stream backing outside
the prepared region. Ordinary hosting keeps the upstream stream setting.

`PreparedServer::with_http2_reset_limits` optionally selects finite counts for
peer-reset pending accepts and locally generated error resets. Omission leaves
the pinned Hyper/h2 defaults intact; zero is passed through with its native
meaning. These controls affect reset retention and GOAWAY behavior, not managed
allocator bytes. The pinned auto builder does not expose h2's separate reset
expiry/count, HPACK-table, or DATA-frame-budget settings. h2's automatic DATA
frame overhead budget follows its selected connection window; native slab,
header, allocator rounding and kernel costs remain separate.

The prepared Program writer enforces `BodyPolicy.chunk_bytes` per emitted data
chunk. Finite and opaque native replies do not pass through that writer; shared
Bytes backing, native copies, headers and final transport settlement are separate
ownership/envelope obligations. Hyper's per-stream send-buffer setting alone is
not a bound on an arbitrary application-supplied chunk or its underlying backing.

For example, a prepared endpoint body can bind a capture using the existing
endpoint grammar (the application's route codec and `render_order` provide the
concrete input and reply types):

```rust,ignore
let plan = endpoint! {
    prepared |order: OrderId| {
        let reply = render_order(order)?;
        reply
    },
    contract: graph_contract,
};
let endpoint = plan.prepare(&memory, region, capacity, frames)?;
let router = router.prepared_input_methods(methods, order_route, endpoint);
```

`PreparedServer::with_connection_tasks` selects a caller-provisioned prepared
provider for connection execution. Its explicit capacity, graph contract,
frame pool, cleanup configuration, and panic pool are separate from the
provider's actual working-set budget; `connection_task_demand` reports the
graph and fixed driver storage and counts the startup Arc controls separately.
On transient graph, terminal, provider, or caller-budget shortage, the selected
path closes the accepted socket before parser or task construction and releases
the partial admission. The accept loop holds at most one unadmitted socket;
kernel and native socket costs remain separately accounted. Failure origins can
remain retained until host-result retirement, so their finite capacity can keep
refusing connections. The selected provider does not itself bound Hyper parser
allocations or captured pointees.

The server uses upstream Hyper and Tokio. Their parser buffers, task allocation,
and other private native storage are not bounded by these endpoint and lifecycle
reservations. Detached work hidden in arbitrary opaque native responses is not
tracked by them. This interface does not establish fixed-budget, startup-only,
or complete transport-settlement hosting. The five rejected local transport forks
and their server-specific executor have been removed.

The optional `server! { http1_header_timeout: duration, ... }` setting supplies
the same caller-selected HTTP/1 header-read deadline to ordinary and prepared
hosting. It installs Hyper's Tokio timer only when selected; its boxed sleeps
remain ordinary runtime allocations, and the selected Tokio runtime must have
its time driver enabled. This deadline does not bound HTTP/2 header
assembly or protocol handshake. Hyper starts it when polling the next HTTP/1
request head, which can include keep-alive waiting; it is not a separately
configured general connection-idle limit.

### Selected native output backing and limits

`PreparedNativeBytesPool` provisions a caller-selected count and size of native
`BytesMut` backings, separately from its region-backed index. Its demand reports
region bytes, requested native backing and startup control counts; the pool also
reports actual native capacities. Standard Arc and Bytes shared-control headers,
allocator overhead and rounding are not included. Startup Vec reservation is
fallible; upstream shared-control promotion and the caller's startup Arc are
infallible native allocations. Install body channels, then select a caller-owned
pool with `PreparedServer::with_native_bytes_pool` to use it for prepared blocking
JSON fragments and SSE encoding. Exhaustion returns an immediate typed memory
refusal; there is no last-alias notification or wait. Existing owned `Bytes`
writes remain moves, and caller-owned SSE strings keep their original backing.

Prepared body JSON codecs also require an explicitly selected
`PreparedJsonWriterPool`, installed after body channels with
`PreparedServer::with_json_writer_pool`. Its caller-selected writer and boundary
capacities own the writer state; its supplied frame pool must accommodate
`PreparedServer::<E, R>::json_writer_sink_layout()`. The selected path reserves
that writer and concrete sink frame before constructing the blocking sink or
taking the value. Missing selection and exhausted backing return typed memory
failures. The ordinary body path retains its existing writer behavior. The
caller accounts for writer-pool and frame-pool demand separately.

`PreparedServer::with_response_limits` optionally selects maximum header field
occurrences and logical name/value bytes, visible bytes per DATA frame, and
field occurrences and logical name/value bytes per trailer frame. Repeated
fields count separately; zero is a valid selected limit. These bounds apply
individually, not to a streaming response's lifetime total. Header checks follow
response hooks and precede publication; DATA and trailer failures retain a typed
`OutputLimit` after commitment. Omission preserves existing behavior.

For responses carrying a prepared endpoint origin, the outer transport adapter
suppresses empty DATA frames and yields after each one. It continues polling the
actual body for EOF, trailers and errors, so completion remains tied to the
body's own outcome. This prevents an always-ready empty DATA stream from filling
HTTP/2's native frame queue without increasing its byte-based send counter. The
ordinary response adapter is unchanged.

Prepared DATA retains its original `Bytes` and endpoint origin in the returned
buffer. Hyper can instead copy bytes into connection-owned storage; that copy
then has its own native lifetime and accounting. In the pinned h2 version a
stream is counted closed only after its pending send queue is empty and buffered
DATA is zero, so queued headers and trailers remain associated with the stream
until that transfer. Header names and values carry their own native owners.
The selected byte pool only reclaims backing when its actual aliases permit it.

These facilities do not bound previously allocated or shared `Bytes` backing,
header construction, Hyper/h2 allocator capacities or wire framing, TCP/kernel
buffering, or final transport settlement. Native body pull and EOF are not proof
of the final copy's retirement. The bounded native working-set envelope and its
measurement remain separate; these options do not certify complete fixed-budget
or allocation-free hosting.
