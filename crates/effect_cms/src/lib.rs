//! Effectful content-management library.
//!
//! The composition root exposes the eleven canonical service capabilities and
//! constructs the definition-service dependency graph. [`definitions`] owns the
//! resource-definition vocabulary, model, builders, registry and relation
//! declarations, [`localization`] owns language normalization, localized
//! drafts and the translation port, [`attachments`] owns the media
//! attachment records and the pure attachment input algebra, [`temporal`] owns
//! date-time and duration values, [`content`] owns content formats, contract
//! payloads, text/asset helpers, sanitization and shortcodes, [`queries`] owns
//! list/detail admission, filter/search values and the scoped relation lookup,
//! [`access`] owns principal/role rules, the management access policy and the
//! guard outcomes, [`validation`] owns the validation issue/path/result contracts,
//! issue localization and the supplied validation binder/validator algebra,
//! [`commands`] owns action availability/support and the consumer-bound command
//! executor map, [`mutations`] owns the W7 mutation progression over those supplied
//! validation and command capabilities together with its mutation-result redirect,
//! [`routing`] owns route sets, route targets, managed route registration/protection,
//! permission metadata synchronization and redirects, [`media`] owns media asset
//! records, consumer asset/attachment store contracts, stored content-media values and the
//! media surface query/state values, [`forms`] owns the form-surface workflow: modes and
//! action filtering, the form page service and frame, section/field state projection,
//! localized tabs and their authoring signals, the localized form controls and the
//! relation editor, [`seo`] owns page metadata, structured data, sitemap documents
//! and the document head, [`rendering`] owns the list/detail page value models,
//! item/section builders, page assembly and rendering services, and [`authoring`]
//! owns localized form translation, the translator/provider contract and the
//! authoring translation route.
//!
//! Boundaries that already govern the port:
//!
//! - Consumers implement every database read and write. The CMS invokes
//!   supplied capabilities and owns no database backend, SQL adapter, schema,
//!   migration, pool or connection.
//! - Effectful behavior uses the workspace `Effect<A, E, R>` model and the
//!   required effectful macros.
//! - Every HTML-producing path uses Maud; manual HTML construction is not
//!   permitted once rendering owners exist.
//!
//! [`add`] is the pre-existing crate scaffold preserved verbatim. It is
//! recorded as preserved inventory, not as a CMS contract.

mod byte_text;
mod json_text;
mod url_text;
mod form_media;
mod composition;

pub use composition::*;

pub mod access;
pub mod attachments;
pub mod authoring;
pub mod commands;
pub mod content;
pub mod definitions;
pub mod forms;
pub mod localization;
pub mod media;
pub mod mutations;
pub mod queries;
pub mod rendering;
pub mod resourcepages;
pub mod routing;
pub mod seo;
pub mod temporal;
pub mod validation;

#[must_use]
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}

pub mod dashboards;

mod text_format;

#[cfg(doctest)]
#[path = "../tests/core_contracts/type_contracts.rs"]
mod core_type_contracts;
