//! Small shared wire models for recorded pickup tickets and queued preparation.
use super::parse_policy;
use effect::{JsonValue, PureCodec, json_i64, json_string, schema, schema_enum};

/// Customer input passed through the workflow and queue codecs.
#[derive(Clone, Debug)]
pub struct Ticket {
    /// Printed pickup number.
    pub number: String,
    /// Name called at pickup.
    pub customer: String,
    /// Ordered cups.
    pub cups: i64,
}
/// Pickup instructions recorded after issuing the ticket labels.
#[derive(Debug)]
pub struct Pickup {
    /// Printed pickup number.
    pub number: String,
    /// Name called at pickup.
    pub customer: String,
    /// Completed cup count.
    pub cups: i64,
}
/// Validation failure recorded alongside a ticket outcome.
#[derive(Debug)]
pub enum TicketError {
    /// This local counter accepts between one and four cups.
    InvalidCups,
}
/// Encode the customer's complete input, without runtime metadata.
pub fn ticket_codec() -> Result<PureCodec<Ticket, JsonValue>, effect::Defect> {
    schema! {
        struct Ticket { number: json_string(), customer: json_string(), cups: json_i64() }
        policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("ticket codec: {error:?}")))
}
/// Record the owned pickup result without requiring Clone.
pub fn pickup_codec() -> Result<PureCodec<Pickup, JsonValue>, effect::Defect> {
    schema! {
        struct Pickup { number: json_string(), customer: json_string(), cups: json_i64() }
        policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("pickup codec: {error:?}")))
}
/// Preserve the domain error as a tagged record.
pub fn error_codec() -> Result<PureCodec<TicketError, JsonValue>, effect::Defect> {
    schema_enum! {
        enum TicketError { InvalidCups => "invalid-cups" }
        tag_field: "problem", content_field: "details", policy: parse_policy()
    }
    .map_err(|error| effect::Defect::new(format!("ticket error codec: {error:?}")))
}
