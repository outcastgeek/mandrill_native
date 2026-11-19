//! Serve an article catalogue through the real Web host and CMS definition services.
//! The example owns its loopback request, response and graceful host drain.

use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use effect::{
    AdmissionCapacity, Cancellation, Effect, EffectEnvironment, EffectErrorLabel, Exit, effect,
    effect_from_result, effect_sync, effect_try_future, service,
};
use effect_cms::definitions::{ResourceDefinition, ResourceDisplayDefinition, ResourceKind};
use effect_cms::rendering::RenderingError;
use effect_cms::resourcepages::ResourcePageRenderer;
use effect_cms::{DefinitionServiceSetOptions, cms};
use effect_server::native::axum::body::Body;
use effect_server::{
    BodyLimit, BodyPolicy, ByteFieldBounds, ByteFormPolicy, Endpoint, HostPolicy, HttpProtocols,
    HttpStatus, Reply, Request, Router, ServerError, endpoint, reply, router, server,
};
use effect_server_web::{WebError, web};
use effect_tokio::{TokioClock, TokioTasks};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;

#[derive(Debug, EffectErrorLabel)]
enum CatalogueFailure {
    #[effect_error_label("catalogue HTTP transport failed")]
    Server(ServerError),
    #[effect_error_label("catalogue Web configuration failed")]
    Web(WebError),
    #[effect_error_label("catalogue client I/O failed")]
    Client(io::Error),
}

impl std::error::Error for CatalogueFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Server(error) => Some(error),
            Self::Web(error) => Some(error),
            Self::Client(error) => Some(error),
        }
    }
}

type PageError = RenderingError<CatalogueFailure>;

struct Providers {
    runtime: Arc<Runtime>,
    clock: TokioClock,
    tasks: TokioTasks,
}

#[derive(EffectEnvironment)]
#[effect_environment(
    request = request,
    cancellation = self.request.cancellation(),
    tokio = self.providers.runtime.handle(),
    clock(provider = TokioClock, get = &self.providers.clock),
    task_spawner(provider = TokioTasks, get = &self.providers.tasks),
)]
struct CatalogueEnv {
    providers: Arc<Providers>,
    request: Request,
}

fn catalogue() -> Endpoint<PageError, CatalogueEnv> {
    let kind = ResourceKind::new("article");
    let mut options = DefinitionServiceSetOptions::new(ByteFormPolicy {
        body: BodyLimit::new(NonZeroUsize::MIN.saturating_add(4_095)),
        fields: NonZeroUsize::MIN.saturating_add(31),
        values: ByteFieldBounds {
            count: NonZeroUsize::MIN.saturating_add(15),
            bytes: NonZeroUsize::MIN.saturating_add(1_023),
        },
    });
    options.default_page_size = 20;
    options.definitions.push(ResourceDefinition {
        kind: kind.clone(),
        display: ResourceDisplayDefinition {
            list_title_key: "Articles".into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let services = cms! { definition_services; options: options, };
    let renderer = ResourcePageRenderer::new(Default::default());
    endpoint! {
        effect: cms! { definition_list_reply; renderer: renderer, services: &services, kind: &kind, },
    }
}

fn render_failure(_: PageError) -> Effect<Reply<PageError, CatalogueEnv>, PageError, CatalogueEnv> {
    effect! {
        let status = effect_from_result! {
            once; result: HttpStatus::new(http::StatusCode::INTERNAL_SERVER_ERROR), on_error: native_failure,
        }?;
        reply! { status: status, text(String::from("Catalogue unavailable")); }
    }
}

fn native_failure(error: ServerError) -> PageError {
    RenderingError::Consumer(CatalogueFailure::Server(error))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
    );
    let providers = Arc::new(Providers {
        clock: TokioClock::new(Arc::clone(&runtime)),
        tasks: TokioTasks::new(
            Arc::clone(&runtime),
            AdmissionCapacity::Bounded(NonZeroUsize::MIN.saturating_add(7)),
        )
        .map_err(|defect| format!("catalogue task provider: {defect:?}"))?,
        runtime: Arc::clone(&runtime),
    });
    let root = Arc::new(CatalogueEnv {
        providers,
        request: Request::from_native(http::Request::new(Body::empty()), Cancellation::new()),
    });
    let routes = router! { get("/articles", catalogue()); }?;
    let policy = server! {
        policy;
        body_policy: BodyPolicy { frames: NonZeroUsize::MIN, chunk_bytes: NonZeroUsize::MIN.saturating_add(4_095) },
        host_policy: HostPolicy { drain: Duration::from_secs(2) },
        native_error: native_failure,
        on_error: render_failure,
    };
    let application = web! {
        policy: policy,
        web_errors: |error| RenderingError::Consumer(CatalogueFailure::Web(error)),
        startup: move |_| {
            let routes = routes.clone();
            effect_sync! { once; body: move || routes, }
        },
        environment: |_, request: Request| service! {
            env: move |root: &CatalogueEnv| Arc::new(CatalogueEnv {
                providers: Arc::clone(&root.providers), request: request.clone(),
            }),
        },
        listen: ([127, 0, 0, 1], 0).into(),
        protocols: HttpProtocols::Http1,
        routes: |routes: Arc<Router<PageError, CatalogueEnv>>| (*routes).clone(),
    };
    let execution = web! {
        with_host; source: application,
        body: |host: effect_server::ServerHost| effect_try_future! {
            once;
            body: move |_| async move {
                let mut client = tokio::net::TcpStream::connect(host.address).await?;
                client.write_all(b"GET /articles HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
                let mut response = Vec::new();
                client.read_to_end(&mut response).await?;
                io::stdout().lock().write_all(&response)
            },
            on_error: |error| RenderingError::Consumer(CatalogueFailure::Client(error)),
        },
    };
    let outcome = runtime.block_on(
        effect! { run_cancellable; program: &execution, environment: Arc::clone(&root), },
    );
    runtime.block_on(root.providers.tasks.stop().wait());
    match outcome {
        Exit::Success(()) => Ok(()),
        other => Err(format!("catalogue host: {other:?}").into()),
    }
}
