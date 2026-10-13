//! Consume three order-status messages over a checked, scope-owned phone socket.
//!
//! Run: `cargo +1.99.0 run -p effect --example phone_order_status`.
//! Use `-- --accepted` to consume the same customer feed on an already accepted socket.
//! Each run follows only one connection: accepted, preparing, ready, then one
//! graceful close and native disposal. The in-process transport supplies the feed;
//! the recipe owns customer messages, bounds and cleanup, without a network service.

use effect::effect_die;
pub mod cafe_support;
use cafe_support::Cafe;
use cafe_support::transport::{PhoneSocket, PhoneTransport};
use effect::websocket::{
    HasWebSocketTransport, WebSocket, WebSocketConnectOptions, WebSocketConnection, WebSocketError,
    WebSocketLimits, WebSocketMessage, WebSocketReceive,
};
use effect::{
    Effect, Exit, ResourceValue, Stream, effect, effect_socket, effect_stream_for_each, stream,
};
use std::num::NonZeroUsize;
use std::{sync::Arc, time::Duration};

/// Await the customer's feed, then finish its close handshake before releasing ownership.
fn read_phone(connection: ResourceValue<WebSocketConnection>) -> Effect<(), WebSocketError, Phone> {
    effect! {
        let connection = connection.clone();
        effect_stream_for_each! {
            stream: messages(connection.clone()),
            body |message: Arc<str>| { { use std::io::Write; let written = writeln!(std::io::stdout().lock(), "Ticket 104: {message}."); match written { Ok(()) => (), Err(error) => effect_die! { defect: effect::Defect::from_io(error), }?, } }; },
        }?;
        let close = connection.access(|socket| socket.close())?;
        close?;
    }
}
/// Read complete text messages until the native peer closes its finite feed.
fn messages(
    connection: ResourceValue<WebSocketConnection>,
) -> Stream<Arc<str>, WebSocketError, Phone> {
    stream! {
        let connection = connection.clone();
        loop {
            let receive = connection.access(|socket| socket.receive())?;
            match receive? {
                WebSocketReceive::Message(WebSocketMessage::Text(message)) => yield message,
                WebSocketReceive::Message(WebSocketMessage::Binary(_)) => effect_die! { defect: effect::Defect::new("unexpected binary status"), }?,
                WebSocketReceive::Closed(_) => break,
            }
        }
    }
}
/// The phone selects its transport beside the same task/cancellation providers.
type Phone = Cafe<PhoneTransport>;
impl HasWebSocketTransport for Phone {
    /// The configured in-process native transport carries these customer messages.
    type Transport = PhoneTransport;
    /// Borrow the selected transport without global discovery.
    fn websocket_transport(&self) -> &PhoneTransport {
        self.services.as_ref()
    }
}
/// Choose the customer's connection boundary; both forms retain identical cleanup.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let statuses = vec![
        Arc::from("accepted"),
        Arc::from("preparing"),
        Arc::from("ready"),
    ];
    let env: Arc<Phone> = Cafe::open(
        PhoneTransport::new(statuses.clone()),
        NonZeroUsize::MIN.saturating_add(15),
        NonZeroUsize::MIN.saturating_add(3),
    );
    let bounds = WebSocketLimits {
        max_message_bytes: NonZeroUsize::MIN.saturating_add(255),
        receive_buffer_bytes: NonZeroUsize::MIN.saturating_add(127),
        close_timeout: Duration::from_secs(1),
    };
    let accepted = PhoneSocket::new(statuses, None);
    let observe = accepted.clone();
    let accepted_mode = std::env::args().any(|arg| arg == "--accepted");
    let program = if accepted_mode {
        effect_socket! {
            socket; limits: bounds,
            acquire { let socket: Arc<dyn WebSocket> = accepted.clone(); socket },
            body: read_phone,
        }
    } else {
        effect_socket! {
            connection; options: WebSocketConnectOptions {
                uri: Arc::from("ws://cafe.local/orders/104"), headers: Arc::new(Default::default()),
                subprotocols: Arc::from([Arc::from("cafe.status")]), limits: bounds,
            },
            body: read_phone,
        }
    };
    let result = futures_executor::block_on(program.run(env.clone()));
    env.base
        .finish()
        .map_err(|error| error.message().to_owned())?;
    match result {
        Exit::Success(()) => (),
        other
        @ (Exit::Fail(_) | Exit::Die(_) | Exit::Interrupted | Exit::FinalizationFailed(_)) => {
            return Err(format!("phone status feed failed: {other:?}").into());
        }
    }
    assert!(if accepted_mode {
        observe.is_released()
    } else {
        env.services
            .all_released()
            .map_err(|error| error.message().to_owned())?
    });
    {
        use std::io::Write;
        writeln!(
            std::io::stdout().lock(),
            "Customer feed closed and released."
        )?;
    };
    Ok(())
}
