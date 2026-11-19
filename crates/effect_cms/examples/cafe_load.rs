//! Run the existing café executable on loopback with a fixed 512 MiB arena.
use chrono::{Duration as DateDuration, Utc};
use effect_server::native::bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs, io,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task::JoinSet,
    time::{sleep, timeout, timeout_at},
};

type TestError = Box<dyn Error + Send + Sync>;
type TestResult<T> = Result<T, TestError>;
const CAPACITY: usize = 32;
const MENU: &str = "neighborhood-cafe: espresso, oat latte; seasonal: Maple oat latte";
const REFUSAL: &str = "The café is busy. Try again shortly.\n";

#[derive(Clone, Copy)]
enum DrainProbe {
    KeepOpen,
    Release,
    Disconnect,
    SaturatedRelease,
}

impl DrainProbe {
    fn name(self) -> &'static str {
        match self {
            Self::KeepOpen => "keep-open",
            Self::Release => "release",
            Self::Disconnect => "disconnect",
            Self::SaturatedRelease => "saturated-release",
        }
    }
}

fn failed(message: impl Into<String>) -> TestError {
    io::Error::other(message.into()).into()
}

fn fixture(folder: &Path, port: u16, queued: bool) -> TestResult<BTreeMap<&'static str, String>> {
    fs::create_dir(folder)?;
    let now = Utc::now();
    let active = (now - DateDuration::days(1)).format("%Y-%m-%dT%H:%M:%SZ");
    let expires = (now + DateDuration::days(30)).format("%Y-%m-%dT%H:%M:%SZ");
    let id = uuid::Uuid::new_v4();
    // A fixture-only key for this fresh loopback service, never a deployment key.
    let key =
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+Pw==";
    let xml = format!(
        r#"<key id="{id}" version="1">
<creationDate>{active}</creationDate>
<activationDate>{active}</activationDate>
<expirationDate>{expires}</expirationDate>
<descriptor deserializerType="AuthenticatedEncryptorDescriptorDeserializer"><descriptor>
<encryption algorithm="AES_256_CBC"/>
<validation algorithm="HMACSHA256"/>
<masterKey><value>{key}</value></masterKey>
</descriptor></descriptor>
</key>"#
    );
    let keys = folder.join("keys.xml");
    let catalog = folder.join("en-US.json");
    fs::write(&keys, xml)?;
    fs::write(
        &catalog,
        r#"{"orders":{"ready":"{customer}, your order is ready."}}"#,
    )?;
    let mut settings = BTreeMap::from([
        ("CAFE_LISTEN", format!("127.0.0.1:{port}")),
        (
            "CAFE_DATABASE",
            folder.join("cafe.db").display().to_string(),
        ),
        ("CAFE_KEY_RING", keys.display().to_string()),
        ("CAFE_ORIGIN", format!("http://127.0.0.1:{port}")),
        ("CAFE_CATALOG", catalog.display().to_string()),
    ]);
    settings.extend(
        [
            ("CAFE_COOKIE", "Cafe.Customer"),
            ("CAFE_COOKIE_SECURE", "false"),
            ("CAFE_CULTURE", "en-US"),
            ("CAFE_WORKERS", "2"),
            ("CAFE_MEMORY_BYTES", "536870912"),
            ("CAFE_CAPACITY", "32"),
            ("CAFE_BYTES", "8192"),
            ("CAFE_DEPTH", "8"),
            ("CAFE_HASH_ITERATIONS", "100000"),
            ("CAFE_WINDOW_SECONDS", "5"),
            ("CAFE_SESSION_SECONDS", "1800"),
            ("CAFE_RECENT_SECONDS", "300"),
            ("CAFE_RETRY_SECONDS", "1"),
        ]
        .map(|(key, value)| (key, value.to_owned())),
    );
    if queued {
        settings.insert("CAFE_QUEUE_CAPACITY", CAPACITY.to_string());
    }
    Ok(settings)
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Response {
    fn expect(&self, status: u16, contains: &str) -> TestResult<()> {
        let body = String::from_utf8_lossy(&self.body);
        if self.status != status || !body.contains(contains) {
            return Err(failed(format!(
                "Expected HTTP {status} containing {contains:?}. Received {}: {body:?}",
                self.status
            )));
        }
        Ok(())
    }
}

async fn send(
    port: u16,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: &'static [u8],
) -> TestResult<Response> {
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
    let (mut client, driver) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    let driver = tokio::spawn(driver);
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, format!("127.0.0.1:{port}"))
        .header(header::CONNECTION, "close")
        .body(Full::new(Bytes::from_static(body)))?;
    request.headers_mut().extend(headers);
    let response = client.send_request(request).await?;
    let (parts, body) = response.into_parts();
    let body = body.collect().await?.to_bytes();
    drop(client);
    driver.await??;
    Ok(Response {
        status: parts.status,
        headers: parts.headers,
        body,
    })
}

#[derive(Default)]
struct Browser {
    cookies: BTreeSet<String>,
}

impl Browser {
    fn cookie_header(&self) -> String {
        self.cookies.iter().cloned().collect::<Vec<_>>().join("; ")
    }

    async fn request(
        &mut self,
        port: u16,
        path: &str,
        post: Option<(&'static [u8], Option<&str>)>,
    ) -> TestResult<Response> {
        let mut headers = HeaderMap::new();
        if !self.cookies.is_empty() {
            headers.insert(header::COOKIE, self.cookie_header().parse()?);
        }
        let (method, body) = match post {
            Some((body, csrf)) => {
                headers.insert(header::ORIGIN, format!("http://127.0.0.1:{port}").parse()?);
                if !body.is_empty() {
                    headers.insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    );
                }
                if let Some(token) = csrf {
                    headers.insert("x-identity-csrf", token.parse()?);
                }
                (Method::POST, body)
            }
            None => (Method::GET, b"".as_slice()),
        };
        let response = timeout(
            Duration::from_secs(3),
            send(port, method, path, headers, body),
        )
        .await??;
        for value in response.headers.get_all(header::SET_COOKIE) {
            let cookie = value.to_str()?.split(';').next().unwrap_or_default();
            if let Some((name, _)) = cookie.split_once('=') {
                self.cookies
                    .retain(|previous| previous.split_once('=').is_none_or(|(old, _)| old != name));
                self.cookies.insert(cookie.to_owned());
            }
        }
        Ok(response)
    }

    async fn csrf(&mut self, port: u16) -> TestResult<String> {
        let reply = self.request(port, "/identity/csrf", None).await?;
        reply.expect(200, "\"token\"")?;
        let payload: Value = serde_json::from_slice(&reply.body)?;
        payload
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| failed("missing native CSRF token"))
    }
}

async fn request_contract(port: u16) -> TestResult<()> {
    let mut browser = Browser::default();
    browser
        .request(port, "/menu", None)
        .await?
        .expect(200, MENU)?;
    browser
        .request(port, "/health", None)
        .await?
        .expect(200, "neighborhood-cafe")?;
    browser
        .request(port, "/pickup", None)
        .await?
        .expect(401, "Sign in to view your own pickup order.")?;
    let token = browser.csrf(port).await?;
    let credentials = br#"{"username":"maya","password":"Cafe-Demo-Pickup-42!"}"#;
    browser
        .request(port, "/identity/login", Some((credentials, Some(&token))))
        .await?
        .expect(200, "Signed in. Your pickup order is ready to view.")?;
    browser
        .request(port, "/pickup", None)
        .await?
        .expect(200, "Maya, your order is ready.")?;
    let collect = "/identity/pickup/collect";
    browser
        .request(port, collect, Some((b"", None)))
        .await?
        .expect(403, "")?;
    let token = browser.csrf(port).await?;
    browser
        .request(port, collect, Some((b"", Some(&token))))
        .await?
        .expect(200, "Collection recorded.")?;
    browser
        .request(port, collect, Some((b"", Some(&token))))
        .await?
        .expect(200, "Already collected; no second collection was recorded.")?;
    browser
        .request(port, "/pickup", None)
        .await?
        .expect(200, "Collected")
}

struct RetainedRequests {
    sockets: Vec<TcpStream>,
    unconfirmed: BTreeMap<String, usize>,
}

async fn retained_requests(port: u16, count: usize) -> TestResult<RetainedRequests> {
    let mut browser = Browser::default();
    let token = browser.csrf(port).await?;
    let cookie = browser.cookie_header();
    if cookie.is_empty() {
        return Err(failed("missing native context cookie"));
    }
    let host = format!("127.0.0.1:{port}");
    let frame: Arc<str> = format!(
        "POST /identity/login HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nCookie: {cookie}\r\nX-Identity-CSRF: {token}\r\nContent-Type: application/json\r\nContent-Length: 128\r\nExpect: 100-continue\r\n\r\n"
    ).into();
    let accepted = b"HTTP/1.1 100 Continue\r\n\r\n";
    let mut retained = RetainedRequests {
        sockets: Vec::with_capacity(count),
        unconfirmed: BTreeMap::new(),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut attempts = count;
    for round in 0..2 {
        let mut connections = JoinSet::new();
        for _ in 0..attempts {
            let frame = frame.clone();
            connections.spawn(async move {
                timeout_at(deadline, async move {
                    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
                    stream.write_all(frame.as_bytes()).await?;
                    // Continue means the admitted handler actually polled this body.
                    let mut reply = [0; b"HTTP/1.1 100 Continue\r\n\r\n".len()];
                    stream.read_exact(&mut reply).await?;
                    if reply.starts_with(b"HTTP/1.1 503 ") {
                        return Ok(None);
                    }
                    if &reply != accepted {
                        return Err(failed(format!(
                            "Expected admitted body read, received {:?}",
                            String::from_utf8_lossy(&reply)
                        )));
                    }
                    stream.write_all(b"{").await?;
                    Ok(Some(stream))
                })
                .await?
            });
        }
        attempts = 0;
        while let Some(connection) = connections.join_next().await {
            match connection
                .map_err(TestError::from)
                .and_then(std::convert::identity)
            {
                Ok(Some(socket)) => retained.sockets.push(socket),
                Ok(None) => {
                    attempts += 1;
                    *retained.unconfirmed.entry("http_503".into()).or_default() += 1;
                }
                Err(error) => *retained.unconfirmed.entry(error.to_string()).or_default() += 1,
            }
        }
        if round == 1 || attempts == 0 {
            break;
        }
        // Give the previous response owner time to retire before topping up.
        sleep(Duration::from_millis(50)).await;
    }
    Ok(retained)
}

async fn refusal(port: u16, queued: bool) -> TestResult<Value> {
    let retained = retained_requests(port, CAPACITY).await?;
    let active = retained.sockets.len();
    if active != CAPACITY {
        return Err(failed(format!(
            "Confirmed {active} of {CAPACITY} body readers before refusal probe: {:?}",
            retained.unconfirmed
        )));
    }
    let start = Instant::now();
    let deadline = Duration::from_millis(if queued { 3500 } else { 300 });
    let outcome = timeout(
        deadline,
        send(port, Method::GET, "/menu", HeaderMap::new(), b""),
    )
    .await;
    drop(retained);
    let seconds = start.elapsed().as_secs_f64();
    match outcome {
        Ok(Ok(response)) => {
            response.expect(503, REFUSAL)?;
            if queued && seconds < 2.5 {
                return Err(failed(format!(
                    "HTTP 503 arrived after {seconds:.3}s, before the configured 2.5s queue wait"
                )));
            }
            Ok(json!({
                "http_status": response.status.as_u16(), "seconds": seconds,
                "active_body_readers": active,
            }))
        }
        Ok(Err(error)) if !queued => {
            Ok(json!({"transport_refusal": error.to_string(), "seconds": seconds}))
        }
        Err(error) if !queued => {
            Ok(json!({"transport_refusal": error.to_string(), "seconds": seconds}))
        }
        Ok(Err(error)) => Err(error),
        Err(error) => Err(error.into()),
    }
}

async fn menu_ready(process: &mut Child, port: u16, seconds: u64) -> TestResult<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(status) = process.try_wait()? {
            return Err(failed(format!(
                "Café exited before readiness with {status}"
            )));
        }
        if let Ok(Ok(reply)) = timeout(
            Duration::from_secs(2),
            send(port, Method::GET, "/menu", HeaderMap::new(), b""),
        )
        .await
            && reply.expect(200, MENU).is_ok()
        {
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }
    Err(failed(
        "Café did not return the expected menu before its readiness deadline",
    ))
}

async fn load(port: u16, workers: usize, seconds: u64) -> TestResult<Value> {
    let started = Instant::now();
    let duration = Duration::from_secs(seconds);
    let mut tasks = JoinSet::new();
    for _ in 0..workers {
        tasks.spawn(async move {
            let mut counts = BTreeMap::<String, u64>::new();
            let mut times = Vec::new();
            while started.elapsed() < duration {
                let start = Instant::now();
                let mut headers = HeaderMap::new();
                headers.insert(
                    "traceparent",
                    HeaderValue::from_static(
                        "00-002233445566778899aabbccddeeff11-1122334455667788-01",
                    ),
                );
                let name = match timeout(
                    Duration::from_secs(3),
                    send(port, Method::GET, "/menu", headers, b""),
                )
                .await
                {
                    Ok(Ok(reply))
                        if reply.status == StatusCode::OK
                            && reply.body.as_ref() == MENU.as_bytes() =>
                    {
                        "http_200".to_owned()
                    }
                    Ok(Ok(reply)) if reply.status == StatusCode::OK => "wrong_menu".to_owned(),
                    Ok(Ok(reply))
                        if reply.status == StatusCode::SERVICE_UNAVAILABLE
                            && reply.body.as_ref() == REFUSAL.as_bytes() =>
                    {
                        "static_503".to_owned()
                    }
                    Ok(Ok(reply)) if reply.status == StatusCode::SERVICE_UNAVAILABLE => {
                        "other_503".to_owned()
                    }
                    Ok(Ok(reply)) => format!("http_{}", reply.status.as_u16()),
                    Ok(Err(_)) => "transport_error".to_owned(),
                    Err(_) => "timeout".to_owned(),
                };
                *counts.entry(name).or_default() += 1;
                if times.len() < 2048 {
                    times.push(1000.0 * start.elapsed().as_secs_f64());
                }
            }
            (counts, times)
        });
    }
    let mut total = BTreeMap::<String, u64>::new();
    let mut times = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let (counts, sample) = joined?;
        for (name, count) in counts {
            *total.entry(name).or_default() += count;
        }
        times.extend(sample);
    }
    times.sort_unstable_by(f64::total_cmp);
    let duration = started.elapsed().as_secs_f64();
    let percentiles = [50, 95, 99].map(|percent| times.get(times.len() * percent / 100).copied());
    Ok(json!({
        "workers": workers, "seconds": duration, "outcomes": total,
        "attempts_per_second": total.values().sum::<u64>() as f64 / duration,
        "latency_samples": times.len(), "p50_p95_p99_ms": percentiles,
    }))
}

fn memory_sample(pid: u32) -> Value {
    if cfg!(target_os = "linux") {
        let process = PathBuf::from(format!("/proc/{pid}"));
        let membership = fs::read_to_string(process.join("cgroup")).unwrap_or_default();
        let membership = membership.lines().find_map(|line| line.strip_prefix("0::"));
        let mut cgroup = BTreeMap::new();
        if let Some(path) = membership {
            let path = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
            for field in [
                "current",
                "peak",
                "high",
                "max",
                "swap.current",
                "swap.max",
                "events",
                "stat",
            ] {
                cgroup.insert(
                    field,
                    fs::read_to_string(path.join(format!("memory.{field}"))).ok(),
                );
            }
        }
        return json!({
            "status": fs::read_to_string(process.join("status")).ok(),
            "smaps_rollup": fs::read_to_string(process.join("smaps_rollup")).ok(),
            "cgroup_path": membership, "cgroup": cgroup,
        });
    }
    let pid = pid.to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=,vsz=", "-p", &pid])
        .output();
    json!({"ps_rss_vsz_kib": output.ok().filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())})
}

fn check_service_scope(samples: &[Value]) -> TestResult<()> {
    let mut confirmed = false;
    for sample in samples {
        let Some(cgroup) = sample.pointer("/memory/cgroup").and_then(Value::as_object) else {
            continue;
        };
        let field = |name| cgroup.get(name).and_then(Value::as_str).map(str::trim);
        if field("high") != Some("1342177280")
            || field("max") != Some("1610612736")
            || field("swap.max") != Some("0")
        {
            continue;
        }
        let Some(events) = field("events") else {
            continue;
        };
        confirmed = true;
        for name in ["oom", "oom_kill"] {
            if !events
                .lines()
                .any(|line| line.split_ascii_whitespace().eq([name, "0"]))
            {
                return Err(failed(format!(
                    "Expected zero {name} events in the café scope"
                )));
            }
        }
    }
    if !confirmed {
        return Err(failed(
            "No Linux sample confirmed the configured service limits and events",
        ));
    }
    Ok(())
}

fn stop(process: &mut Child, after_signal: impl FnOnce()) -> TestResult<Value> {
    let start = Instant::now();
    if process.try_wait()?.is_none() {
        // Use the host's signal utility directly. No shell or script is involved.
        let pid = process.id().to_string();
        let status = Command::new("kill").args(["-INT", &pid]).status()?;
        if !status.success() {
            return Err(failed("Unable to signal the café process"));
        }
        after_signal();
    }
    loop {
        if let Some(status) = process.try_wait()? {
            return Ok(
                json!({"status": status.to_string(), "success": status.success(),
                            "seconds": start.elapsed().as_secs_f64()}),
            );
        }
        if start.elapsed() >= Duration::from_secs(15) {
            process.kill()?;
            let status = process.wait()?;
            return Ok(
                json!({"status": status.to_string(), "success": false, "forced": true,
                            "seconds": start.elapsed().as_secs_f64()}),
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

async fn execute(
    process: &mut Child,
    port: u16,
    queued: bool,
    seconds: u64,
    phase: &Mutex<&'static str>,
    stages: &mut BTreeMap<&'static str, Value>,
) -> TestResult<Vec<TcpStream>> {
    menu_ready(process, port, 30).await?;
    *phase.lock().unwrap_or_else(PoisonError::into_inner) = "request_contract";
    request_contract(port).await?;
    stages.insert("request_contract", json!({"passed": true}));
    *phase.lock().unwrap_or_else(PoisonError::into_inner) = "retained_requests";
    stages.insert("forced_refusal", refusal(port, queued).await?);
    menu_ready(process, port, 10).await?;
    stages.insert("recovered_after_refusal", json!(true));
    for (name, workers) in [("steady", 2), ("overload", CAPACITY * 2), ("recovery", 2)] {
        *phase.lock().unwrap_or_else(PoisonError::into_inner) = name;
        let result = load(port, workers, seconds).await?;
        let unexpected = result
            .get("outcomes")
            .and_then(Value::as_object)
            .is_some_and(|counts| {
                counts.keys().any(|key| {
                    key == "wrong_menu" || (key.starts_with("http_") && key != "http_200")
                })
            });
        let valid = result
            .pointer("/outcomes/http_200")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        stages.insert(name, result);
        if unexpected || (name == "recovery" && valid == 0) {
            return Err(failed(format!(
                "Invalid responses during {name}; inspect the recorded outcomes"
            )));
        }
    }
    menu_ready(process, port, 10).await?;
    stages.insert("recovered_after_load", json!(true));
    *phase.lock().unwrap_or_else(PoisonError::into_inner) = "shutdown_pressure";
    let retained = retained_requests(port, CAPACITY).await?;
    stages.insert(
        "connections_held_for_shutdown",
        json!(retained.sockets.len()),
    );
    stages.insert("unconfirmed_before_shutdown", json!(retained.unconfirmed));
    Ok(retained.sockets)
}

fn run(
    binary: &Path,
    evidence: &Path,
    port: u16,
    queued: bool,
    seconds: u64,
    linux_service_limits: bool,
    probe: Option<DrainProbe>,
) -> TestResult<()> {
    fs::create_dir(evidence)?;
    let evidence = evidence.canonicalize()?;
    let settings = fixture(&evidence.join("state"), port, queued)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let log = fs::File::create(evidence.join("service.log"))?;
    let mut command = if linux_service_limits {
        let mut command = Command::new("systemd-run");
        command.args([
            "--user",
            "--scope",
            "--quiet",
            "--property=MemoryHigh=1342177280",
            "--property=MemoryMax=1610612736",
            "--property=MemorySwapMax=0",
        ]);
        command.arg(binary);
        command
    } else {
        Command::new(binary)
    };
    let mut service = command
        .env_remove("CAFE_QUEUE_CAPACITY")
        .envs(&settings)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let pid = service.id();
    let mut stages = BTreeMap::new();
    let start = Instant::now();
    let phase = Arc::new(Mutex::new("startup"));
    let done = Arc::new(AtomicBool::new(false));
    let samples = {
        let phase = phase.clone();
        let done = done.clone();
        thread::spawn(move || {
            let mut samples = Vec::new();
            while !done.load(Ordering::Acquire) {
                let current = *phase.lock().unwrap_or_else(PoisonError::into_inner);
                samples.push(
                    json!({"seconds": start.elapsed().as_secs_f64(), "phase": current,
                                    "memory": memory_sample(pid)}),
                );
                thread::sleep(Duration::from_millis(500));
            }
            samples
        })
    };

    let mut result = runtime.block_on(async {
        if let Some(probe) = probe {
            menu_ready(&mut service, port, 30).await?;
            *phase.lock().unwrap_or_else(PoisonError::into_inner) = "shutdown_pressure";
            let count = if matches!(probe, DrainProbe::SaturatedRelease) {
                CAPACITY
            } else {
                1
            };
            let retained = retained_requests(port, count).await?;
            if count == 1 && retained.sockets.len() != count {
                return Err(failed(format!(
                    "Could not confirm the admitted body read: {:?}",
                    retained.unconfirmed
                )));
            }
            if matches!(probe, DrainProbe::Disconnect) {
                drop(retained.sockets);
                sleep(Duration::from_millis(100)).await;
                menu_ready(&mut service, port, 10).await?;
                stages.insert("recovered_after_disconnect", json!(true));
                Ok(Vec::new())
            } else {
                stages.insert(
                    "connections_held_for_shutdown",
                    json!(retained.sockets.len()),
                );
                stages.insert("unconfirmed_before_shutdown", json!(retained.unconfirmed));
                Ok(retained.sockets)
            }
        } else {
            execute(&mut service, port, queued, seconds, &phase, &mut stages).await
        }
    });
    *phase.lock().unwrap_or_else(PoisonError::into_inner) = "shutdown";
    let shutdown = stop(&mut service, || {
        if !matches!(probe, Some(DrainProbe::KeepOpen))
            && let Ok(retained) = &mut result
        {
            thread::sleep(Duration::from_millis(100));
            retained.clear();
        }
    });
    if shutdown.is_err() {
        let _ = service.kill();
        let _ = service.wait();
    }
    done.store(true, Ordering::Release);
    let samples = samples
        .join()
        .map_err(|_| failed("memory sample thread failed"))?;
    let scope = if linux_service_limits {
        check_service_scope(&samples)
    } else {
        Ok(())
    };
    let saturated =
        (probe.is_none() || matches!(probe, Some(DrainProbe::SaturatedRelease))).then(|| {
            stages
                .get("connections_held_for_shutdown")
                .and_then(Value::as_u64)
                == Some(CAPACITY as u64)
        });
    let report = json!({
        "host_os": std::env::consts::OS, "host_arch": std::env::consts::ARCH,
        "binary": binary, "linux_service_limits": linux_service_limits,
        "drain_probe": probe.map(DrainProbe::name),
        "settings": settings, "stages": stages, "memory_samples": samples,
        "failure": result.as_ref().err().map(ToString::to_string),
        "scope_failure": scope.as_ref().err().map(ToString::to_string),
        "saturated_for_shutdown": saturated,
        "shutdown": shutdown.as_ref().ok(),
        "shutdown_failure": shutdown.as_ref().err().map(ToString::to_string),
    });
    let report_path = evidence.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", report_path.display());
    // The deadline probe alone retains its stalled client through process exit.
    drop(result?);
    scope?;
    if shutdown?.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(failed("Café did not complete normal SIGINT drain"));
    }
    if saturated == Some(false) {
        return Err(failed(
            "Shutdown stage did not demonstrate the configured active capacity",
        ));
    }
    Ok(())
}

fn main() -> TestResult<()> {
    let mut arguments = std::env::args_os().skip(1);
    let usage = "Usage: cafe_load <cafe-binary> <new-evidence-directory> [--queued] [--seconds N] [--port N] [--linux-service-limits] [--drain-probe keep-open|release|disconnect|saturated-release]";
    let binary = PathBuf::from(arguments.next().ok_or_else(|| failed(usage))?).canonicalize()?;
    let evidence = PathBuf::from(arguments.next().ok_or_else(|| failed(usage))?);
    let (mut port, mut queued, mut seconds) = (8082, false, 30);
    let mut linux_service_limits = false;
    let mut probe = None;
    while let Some(flag) = arguments.next() {
        match flag.to_str() {
            Some("--queued") => queued = true,
            Some("--linux-service-limits") if cfg!(target_os = "linux") => {
                linux_service_limits = true;
            }
            Some("--drain-probe") => {
                probe = match arguments
                    .next()
                    .as_deref()
                    .and_then(std::ffi::OsStr::to_str)
                {
                    Some("keep-open") => Some(DrainProbe::KeepOpen),
                    Some("release") => Some(DrainProbe::Release),
                    Some("disconnect") => Some(DrainProbe::Disconnect),
                    Some("saturated-release") => Some(DrainProbe::SaturatedRelease),
                    _ => return Err(failed(usage)),
                };
            }
            Some("--port") => {
                port = arguments
                    .next()
                    .and_then(|value| value.to_str()?.parse().ok())
                    .filter(|port| *port > 0)
                    .ok_or_else(|| failed(usage))?;
            }
            Some("--seconds") => {
                seconds = arguments
                    .next()
                    .and_then(|value| value.to_str()?.parse().ok())
                    .filter(|seconds| *seconds > 0)
                    .ok_or_else(|| failed(usage))?;
            }
            _ => return Err(failed(usage)),
        }
    }
    run(
        &binary,
        &evidence,
        port,
        queued,
        seconds,
        linux_service_limits,
        probe,
    )
}
