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

Run from the workspace root (the directory containing the root `Cargo.toml`) with Rust 1.99.0 installed. `cargo run` also builds. Package: `effect-server-web`; required features: `identity,otel,sqlx-turso`; no positional arguments. Python 3 creates fresh native XML keys and a JSON translation catalog. Use a free port and keep `CAFE_ORIGIN` consistent with `CAFE_LISTEN`.

```sh
cargo +1.99.0 build --locked -p effect-server-web --features identity,otel,sqlx-turso --example cafe_application
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
CAFE_CAPACITY=32 \
CAFE_BYTES=8192 \
CAFE_DEPTH=8 \
CAFE_HASH_ITERATIONS=100000 \
CAFE_WINDOW_SECONDS=5 \
CAFE_SESSION_SECONDS=1800 \
CAFE_RECENT_SECONDS=300 \
CAFE_RETRY_SECONDS=1 \
cargo +1.99.0 run --locked -p effect-server-web --features identity,otel,sqlx-turso --example cafe_application
```

The setup creates a fresh directory, random 64-byte native master key with a UUID and valid dates, English pickup catalog, and database filename. Startup seeds the native accounts and owner-bound ready orders only if absent. The HTML pages use English labels; the pickup announcement uses the selected catalog’s `orders.ready` template with its typed `customer` argument. Editing that template is observable on a ready customer’s next pickup-page load after the five-second catalog reload interval. It does not change the public menu or reset a collected order.

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

Every environment input is required by [the host wiring](cafe_host/mod.rs):

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
| `CAFE_DEPTH` | Positive catalog nesting-depth bound. |
| `CAFE_HASH_ITERATIONS` | Native password-hash iteration count. |
| `CAFE_WINDOW_SECONDS` | Positive seconds for cache retention, catalog reload, probe deadlines and request/provider drain. |
| `CAFE_SESSION_SECONDS` | Positive cookie/ticket and verified-self grant lifetime in seconds; `1800` gives 30 minutes for the browser workflow. |
| `CAFE_RECENT_SECONDS` | Recent-authentication grant lifetime in seconds. |
| `CAFE_RETRY_SECONDS` | Positive Retry-After guidance in whole seconds. |

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
