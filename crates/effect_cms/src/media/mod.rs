//! Media: shared asset records, consumer store contracts, stored content-media
//! values and surface query/state data.
//!
//! The media children are semantic owners:
//!
//! - [`assets`] owns media asset records and form input, list results, asset and
//!   attachment store contracts, attachment preparation options, normalization and
//!   the default media resource definition;
//! - [`contentmedia`] owns the stored content-media item/video values and their
//!   JSON and attachment-derived transformations;
//! - [`surface`] owns the media query and surface request/state values;
//! - [`attachmentflow`] owns the store-backed attachment preview, listing,
//!   current-input selection, ordered owner replacement and derived mutation media
//!   state over supplied asset and attachment capabilities.
//!
//! # Consumer-owned persistence
//!
//! Every asset and attachment store is a consumer-supplied capability. The CMS
//! declares the typed contracts and invokes them through lazy effects; it owns no
//! media table, SQL statement, schema, migration, pool, connection, backend or
//! storage lifecycle. Consumer stores perform all reads and writes.
//!
//! # Scope
//!
//! The children implement the media foundation, management and fragment workflows,
//! source presentation and consumer-backed demo seeding. Presentation owns Maud
//! markup; the composition module itself produces no HTML and opens no store.

pub mod assets;
pub mod attachmentflow;
pub mod contentmedia;
pub mod management;
pub mod runtime;
pub mod surface;

pub mod demo;
pub mod presentation;

pub mod flows;
