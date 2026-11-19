# Sign in and collect your café order

This example is a small browser workflow: sign in as Maya, see her two ready oat lattes on ticket 104, confirm the drinks were handed over, then reload to see **Collected**. Collection is stored in the selected database. Repeating the request does not record another collection, and restarting with the same database does not reset it.

## Try it in your browser

1. Run the setup below in a terminal and leave the host running. Use port `8082` so an older example on `8081` can keep running independently.
2. Open <http://127.0.0.1:8082/>. The page explains the task and displays a labeled sign-in form.
3. Enter username **maya** and password **Cafe-Demo-Pickup-42!**, then select **Sign in and view my order**. These are deliberately seeded local-demo credentials. Native Identity validates and hashes passwords and issues the protected customer cookie; the form does not bypass authentication.
4. The pickup page shows `Maya, your order is ready.`, ticket **104**, **Oat latte**, **2** cups, and status **Ready**.
5. Check **I have received all my drinks**, then select **Confirm collection**. The page shows **Collected** and `Collection recorded. Enjoy your drinks!`.
6. Select **Reload my order**. Status remains **Collected** and the collection form is absent. Restarting the host with the same database also retains that state. To replay from Ready, run the setup again to obtain a fresh database directory.

Incorrect credentials produce a visible error and retain entered values. Pending actions announce progress and prevent duplicate submission. Sessions last 30 minutes with this recipe. If a session expires, return to the entry page and sign in again.

For a second ownership fixture, username **lee** uses the same demo password and has only ticket **105**, one espresso. Lee cannot see or collect Maya’s order. There is no registration, ordering or payment interface; the ready orders are seeded demonstration data.

`/menu` remains a public text menu, and `/health`, `/ready`, `/live` (plus their `z` aliases) retain operational probes. Keep the host terminal running while using the pages. Press Ctrl-C there when finished; requests, providers and the database pool drain within the explicit cleanup budget.

## Build and start

Run from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.100.0 installed. `cargo run` also builds. Package: `effect-server-web`; required features: `identity,otel,sqlx-turso`; no positional arguments. Python 3 creates fresh native XML keys and a JSON translation catalog. Use a free port and keep `CAFE_ORIGIN` consistent with `CAFE_LISTEN`.

```sh
cargo +1.100.0 build --locked -p effect-server-web --features identity,otel,sqlx-turso --example cafe_application
web_dir=$(mktemp -d "${TMPDIR:-/tmp}/effect-web.XXXXXX")
python3 - "$web_dir" <<'PYSETUP'
import base64
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import secrets
import sys
import uuid

folder = Path(sys.argv[1])
now = datetime.now(timezone.utc)
active = (now - timedelta(days=1)).strftime('%Y-%m-%dT%H:%M:%SZ')
expires = (now + timedelta(days=365)).strftime('%Y-%m-%dT%H:%M:%SZ')
master_key = base64.b64encode(secrets.token_bytes(64)).decode('ascii')
xml = f'''<key id="{uuid.uuid4()}" version="1">
  <creationDate>{active}</creationDate>
  <activationDate>{active}</activationDate>
  <expirationDate>{expires}</expirationDate>
  <descriptor deserializerType="AuthenticatedEncryptorDescriptorDeserializer">
    <descriptor>
      <encryption algorithm="AES_256_CBC"/>
      <validation algorithm="HMACSHA256"/>
      <masterKey><value>{master_key}</value></masterKey>
    </descriptor>
  </descriptor>
</key>
'''
(folder / 'keys.xml').write_text(xml, encoding='utf-8')
(folder / 'en-US.json').write_text(
    json.dumps({'orders': {'ready': '{customer}, your order is ready.'}}),
    encoding='utf-8',
)
PYSETUP
CAFE_LISTEN=127.0.0.1:8082 \
CAFE_DATABASE="$web_dir/cafe.db" \
CAFE_KEY_RING="$web_dir/keys.xml" \
CAFE_ORIGIN=http://127.0.0.1:8082 \
CAFE_COOKIE=Cafe.Customer \
CAFE_COOKIE_SECURE=false \
CAFE_CULTURE=en-US \
CAFE_CATALOG="$web_dir/en-US.json" \
CAFE_WORKERS=2 \
CAFE_MEMORY_BYTES=536870912 \
CAFE_CAPACITY=32 \
CAFE_BYTES=8192 \
CAFE_DEPTH=8 \
CAFE_HASH_ITERATIONS=100000 \
CAFE_WINDOW_SECONDS=5 \
CAFE_SESSION_SECONDS=1800 \
CAFE_RECENT_SECONDS=300 \
CAFE_RETRY_SECONDS=1 \
cargo +1.100.0 run --locked -p effect-server-web --features identity,otel,sqlx-turso --example cafe_application
```

The setup creates a fresh directory, random 64-byte native master key with a UUID and valid dates, English pickup catalog, and database filename. Startup seeds the native accounts and owner-bound ready orders only if absent. The HTML pages use English labels; the pickup announcement uses the selected catalog’s `orders.ready` template with its typed `customer` argument. Editing that template is observable on a ready customer’s next pickup-page load after the five-second catalog reload interval. It does not change the public menu or reset a collected order.

The command provisions a single 512 MiB managed region and selects immediate overload refusal. Add `CAFE_QUEUE_CAPACITY=32` to the same command to also allow up to 32 parsed heads to wait for one of the 32 active request places. The queue reserves its FIFO and wake records at startup from the same arena, and the host allows 32 additional connections in that mode. A waiting head holds no Effectful request-body, decode or response buffer. A full queue or wait expiry uses the existing static HTTP 503 response while the connection remains open. The optional queue wait uses half the configured window, leaving time for the reply before the HTTP idle deadline. Earlier connection exhaustion can leave the socket unaccepted. Hyper and the network can retain native input independently.

## Linux configuration for a 2 GB host

This worked starting profile uses a **2,000,000,000-byte host budget** and keeps the same **536,870,912-byte (512 MiB) Effectful arena** as the launch above. The systemd unit uses Linux cgroup v2. Its high threshold starts reclaim and throttling. The maximum is the final containment boundary and can cause a cgroup OOM kill. The café's request limits, deadlines and static 503 response are the application-level overload controls. The numbers below are provisional configuration values. Measure the envelope on the target host before relying on them. [systemd memory controls](https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html#Memory%20Accounting%20and%20Control) · [Linux memory controller](https://docs.kernel.org/admin-guide/cgroup-v2.html#memory-interface-files)

| Budget component | Bytes | Accounting boundary |
| --- | ---: | --- |
| Effectful arena | 536,870,912 | One managed reservation, including region overhead and unused space. |
| Native service allowance | 805,306,368 | Non-region memory and service margin below the high threshold. |
| Service surge allowance | 268,435,456 | Difference between the high threshold and the maximum. |
| Outside-service host reserve | 389,387,264 | Kernel and system services outside the unit, including any separate proxy. |
| **Host budget** | **2,000,000,000** | Recalculate the split if the machine’s usable `MemTotal` is smaller. |

Use the existing café executable as `/opt/effectful/bin/cafe_application` under the dedicated `effectful-cafe` service account. Give that account access to the catalogue and key files created in the local setup, and let it own the database directory. This fixture listens on loopback with the same local HTTP settings as the command above. If you add a TLS proxy, include its memory in the host worksheet.

`/etc/systemd/system/effectful-cafe.service`:

```ini
[Unit]
Description=Effectful café example

[Service]
Type=exec
User=effectful-cafe
StateDirectory=effectful-cafe
EnvironmentFile=/etc/effectful/cafe.env
ExecStart=/opt/effectful/bin/cafe_application
MemoryAccounting=yes
MemoryHigh=1342177280
MemoryMax=1610612736
MemorySwapMax=0
KillSignal=SIGINT
TimeoutStopSec=30s
```

`SIGINT` reaches the café's existing Ctrl-C handler so it can drain request and provider work. The service stop deadline is the outer process limit. The unit starts the executable directly, keeping Cargo and compilation outside service accounting. [systemd execution settings](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html) · [systemd stop signals](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html)

`/etc/effectful/cafe.env`:

```ini
CAFE_LISTEN=127.0.0.1:8082
CAFE_DATABASE=/var/lib/effectful-cafe/cafe.db
CAFE_KEY_RING=/var/lib/effectful-cafe/keys.xml
CAFE_ORIGIN=http://127.0.0.1:8082
CAFE_COOKIE=Cafe.Customer
CAFE_COOKIE_SECURE=false
CAFE_CULTURE=en-US
CAFE_CATALOG=/var/lib/effectful-cafe/en-US.json
CAFE_WORKERS=2
CAFE_MEMORY_BYTES=536870912
CAFE_CAPACITY=32
CAFE_BYTES=8192
CAFE_DEPTH=8
CAFE_HASH_ITERATIONS=100000
CAFE_WINDOW_SECONDS=5
CAFE_SESSION_SECONDS=1800
CAFE_RECENT_SECONDS=300
CAFE_RETRY_SECONDS=1
```

The environment file selects immediate refusal. Set `CAFE_QUEUE_CAPACITY=32` to use the bounded queue variant. That variant keeps the arena total and increases the admitted connection count, so include its native costs in the worksheet when qualifying it.

### Non-region memory worksheet

Record startup, steady load, overload and drained peaks for the chosen profile. Use aggregate cgroup charge to check the service envelope. Use the component figures to explain that total, since RSS/PSS, mapped files and cgroup counters overlap.

| Domain | Evidence to record |
| --- | --- |
| Managed arena | Plan total, committed/resident arena memory where observable, and region used/backing high-water marks. |
| Native heap and allocator | Reservations and committed/resident usage, including allocator retention, Tokio, Hyper, Identity, OTel/export and any in-unit TLS. |
| Threads | Reactor, worker and blocking thread counts, stack reservations and resident stack pages. |
| Database and executable | SQL pager, connection and mapped database memory where present, charged file cache, and executable/shared-library mappings. |
| Kernel and network | In-unit socket buffers, page tables, kernel stacks and slab. Record outside-unit network costs and other services against the host reserve. |
| Process, service and host totals | Process RSS/PSS from `/proc/<pid>/smaps_rollup`; unit `memory.current`, `memory.peak` where available, `memory.stat`, `memory.events` and `memory.swap.current`; host `MemTotal` and `MemAvailable`. |

For a total-service target, pick an arena size that satisfies **arena reservation + measured peak non-region charge + service margin ≤ effective high threshold** and still fits the application's checked region plan. Use measurements from the same workload and profile, and account for any lower parent-cgroup limit. Keep the hard maximum plus the independently measured outside-service reserve within usable host memory. Validate graceful refusal and recovery by observing the application's overload replies, cleanup progress and memory plateau alongside cgroup events. Treat OOM events as a failed overload qualification. Measurements and load qualification are separate from this configuration. [Linux accounting fields](https://docs.kernel.org/admin-guide/cgroup-v2.html#memory-interface-files) · [Process memory accounting](https://docs.kernel.org/filesystems/proc.html)

## Native request flow and curl verification

The browser performs these requests internally. Customers use the forms above; they do not need to construct JSON or tokens. Native server-backed browser context binds each CSRF token to the current session cookie, so the browser obtains a fresh token after login before collection. Collection uses a POST under `/identity`, matching the native context cookie’s path.

For a terminal check against a **fresh** setup, use a second terminal:

```sh
curl -i http://127.0.0.1:8082/pickup
curl -i http://127.0.0.1:8082/menu
curl -i http://127.0.0.1:8082/health
```

Anonymous `/pickup` returns HTTP **401** with an HTML error page saying `Sign in to view your own pickup order.` and linking back to café sign-in. The menu returns HTTP 200 and `neighborhood-cafe: espresso, oat latte; seasonal: Maple oat latte`. Health returns HTTP 200 JSON identifying `neighborhood-cafe` and its health role.

To exercise native login and collection using curl (Python 3 extracts only the native token):

```sh
check_dir=$(mktemp -d "${TMPDIR:-/tmp}/effect-pickup-check.XXXXXX")
curl -fsS -c "$check_dir/cookies" http://127.0.0.1:8082/identity/csrf > "$check_dir/csrf.json"
token=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["token"])' "$check_dir/csrf.json")
curl -i -b "$check_dir/cookies" -c "$check_dir/cookies" -H 'Origin: http://127.0.0.1:8082' -H 'Content-Type: application/json' -H "X-Identity-CSRF: $token" --data '{"username":"maya","password":"Cafe-Demo-Pickup-42!"}' http://127.0.0.1:8082/identity/login
curl -i -b "$check_dir/cookies" http://127.0.0.1:8082/pickup
curl -fsS -b "$check_dir/cookies" -c "$check_dir/cookies" http://127.0.0.1:8082/identity/csrf > "$check_dir/csrf.json"
token=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["token"])' "$check_dir/csrf.json")
curl -i -b "$check_dir/cookies" -H 'Origin: http://127.0.0.1:8082' -H "X-Identity-CSRF: $token" -X POST http://127.0.0.1:8082/identity/pickup/collect
curl -i -b "$check_dir/cookies" http://127.0.0.1:8082/pickup
curl -i -b "$check_dir/cookies" -H 'Origin: http://127.0.0.1:8082' -H "X-Identity-CSRF: $token" -X POST http://127.0.0.1:8082/identity/pickup/collect
```

Successful login returns HTTP 200 with `Signed in. Your pickup order is ready to view.` and a native customer cookie. First collection returns HTTP 200 with `Collection recorded. Enjoy your drinks!`. The repeat returns HTTP 200 with `Already collected; no second collection was recorded.`. Reloaded HTML shows `Collected`. A missing/invalid CSRF token or untrusted Origin is rejected before mutation.

## Configuration reference

[The host wiring](cafe_host/mod.rs) reads these environment inputs.

| Variable | Meaning |
| --- | --- |
| `CAFE_LISTEN` | Listener IP address and port; select a free port. |
| `CAFE_DATABASE` | Local native SQLx/Turso database filename. |
| `CAFE_KEY_RING` | UTF-8 native data-protection XML key file, not JSON. |
| `CAFE_ORIGIN` | Allowed browser origin, matching the selected address and scheme. |
| `CAFE_COOKIE` | Selected customer session-cookie name. |
| `CAFE_COOKIE_SECURE` | Boolean; `false` permits this local HTTP recipe. |
| `CAFE_CULTURE` | Declared UI/formatting culture matching the catalog. |
| `CAFE_CATALOG` | UTF-8 JSON translation file. |
| `CAFE_WORKERS` | Positive native Tokio worker-thread count. |
| `CAFE_CAPACITY` | Positive count bound for admission, cache entries/loaders, catalog keys, probes and telemetry capacities. |
| `CAFE_BYTES` | Positive byte bound for finite resources, key file, body chunks, catalog keys/values/messages and cached responses. |
| `CAFE_MEMORY_BYTES` | Optional total for the shared Effectful arena, in bytes. Use `536870912` for 512 MiB. When omitted, the host uses the checked sum of the configured regions. Startup refuses a total smaller than the region requests. |
| `CAFE_QUEUE_CAPACITY` | Optional positive number of waiting request heads. Omit it for immediate refusal. |
| `CAFE_DEPTH` | Positive catalog nesting-depth bound. |
| `CAFE_HASH_ITERATIONS` | Native password-hash iteration count. |
| `CAFE_WINDOW_SECONDS` | Positive seconds for cache retention, catalog reload, probe deadlines, first HTTP head, HTTP/1 header read, HTTP network idle and request/provider drain. The optional queue wait uses half the window. |
| `CAFE_SESSION_SECONDS` | Positive cookie/ticket and verified-self grant lifetime in seconds; `1800` gives 30 minutes for the browser workflow. |
| `CAFE_RECENT_SECONDS` | Recent-authentication grant lifetime in seconds. |
| `CAFE_RETRY_SECONDS` | Positive Retry-After guidance in whole seconds. |

The arena covers the café's prepared origin and dependency caches, finite output cache and retained response backing, availability report workspace, catalogue generation slots, host request and connection controls, Identity and telemetry admission controls, Identity diagnostic carriers, and the SQL session, cleanup and row backing used by the readiness and pickup reads. The optional FIFO is charged only when selected. Native runtime, HTTP parser storage, database connections, Identity providers, telemetry SDK and application callback allocations retain their own ownership domains.

## Local load and refusal reproduction

Build the café, then run the Rust loopback driver from the CMS fixture crate. It uses the existing Hyper HTTP/1 client, starts a fresh café with a 512 MiB arena, and records native login and collection, retained-request refusal, steady and overload traffic, recovery, and SIGINT drain.

```sh
cargo build --locked -p effect-server-web --features identity,otel,sqlx-turso --example cafe_application
cargo run --locked -p effect_cms --example cafe_load -- target/debug/examples/cafe_application /tmp/cafe-immediate-evidence
cargo run --locked -p effect_cms --example cafe_load -- target/debug/examples/cafe_application /tmp/cafe-queued-evidence --queued
```

Each run needs a fresh evidence directory and a free loopback port. Use `--port N` to select another port and `--seconds N` to set the duration of each load phase. The driver writes the service log and raw process samples to that directory. On Linux, `--linux-service-limits` runs the café in a transient user scope with the same high, max and swap values as the deployment unit. The load driver stays outside that scope. The Linux report includes the process's actual cgroup readings. Record the host's own memory envelope separately when qualifying the complete 2 GB deployment.

The driver counts a stalled request only after Hyper reports HTTP 100 Continue, which confirms the handler started reading its body. The normal saturated shutdown exercise releases these clients 100 ms after SIGINT. Use `--drain-probe saturated-release` to run that shutdown alone. Use `--drain-probe keep-open` to hold one admitted body until the five-second deadline and record the typed cleanup failure, or `--drain-probe release` to release one client before the deadline.

The public menu caches anonymous finite replies. Requests with authentication or cookies bypass that cache and use the same renderer limit. The host budgets separate output slots for retained menu replies and live requests. The welcome and pickup pages render into an admitted output slot, and stock overload and error replies have reusable static backing.

The [application](cafe_application.rs) composes existing Web caches, localization, security and observability. [Native host wiring](cafe_host/mod.rs) supplies the owned services; [customer routes](cafe_host/customer_routes.rs) bind existing Identity workflows; [pickup data/workflow](pickup/mod.rs) uses typed owner-bound SQL and a single atomic state change. The support directories are not separate runnable targets. No external telemetry collector is required. See [all example guides](../../effect/examples/README.md#support-modules-and-extension-examples).

Identity routes use `identity_endpoints!` input/workflow/render blocks. A single
`protect(RoleClaimAuthorizationPolicy::require_role("CUSTOMER"), pickup_routes)`
protects the pickup collection. The view requires `pickup.read`; collection requires
`pickup.collect`. Native role membership and permission claims are seeded for the
two local accounts. Endpoint policies add to the group policy, while resource guards
retain actor ownership and exact action/resource grants. Authentication challenges
return 401; insufficient roles or permissions return 403. Login and CSRF issuance
are separate public entry routes. Native CSRF proof remains required for collection.
The native factory and authorization blocks infer their selected concrete types.
