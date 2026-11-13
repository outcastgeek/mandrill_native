//! The detail page view models.
//!
//! The field value is the forms owner's [`ResourceFieldVM`]; no second field
//! contract is introduced. Sections, links, extras and the aggregate page keep the
//! source's field names and order. The extras and link maps are shared handles so a
//! copied state/page observes an in-place map mutation, while each link value is
//! copied with the source's ordinary value semantics.

use crate::definitions::{ResourceRouteSet, ResourceSectionKey, ResourceViewMode};
use crate::definitions::SharedMap;
use crate::seo::SEOVM;

use super::contracts::SEOView;
use super::list_vms::{BreadcrumbVM, ResourceListItemVM};

pub use crate::forms::ResourceFieldVM as FieldVM;

/// One rendered detail section.
#[derive(Clone, Debug, Default)]
pub struct SectionVM {
    /// Section key.
    pub key: ResourceSectionKey,
    /// Localized section title.
    pub title: String,
    /// Localized section description.
    pub description: String,
    /// Section body.
    pub body: String,
    /// Whether the body is authored HTML.
    pub body_is_html: bool,
    /// Renderable fields.
    pub fields: Vec<FieldVM>,
    /// Renderable items.
    pub items: Vec<ResourceListItemVM>,
}

/// One rendered link.
#[derive(Clone, Debug, Default)]
pub struct LinkVM {
    /// Localized label.
    pub label: String,
    /// Link target.
    pub href: String,
    /// Accessible label.
    pub aria_label: String,
    /// Link relation.
    pub rel: String,
    /// Preview image target.
    pub image_url: String,
    /// Preview image alt text.
    pub image_alt: String,
    /// Summary text.
    pub summary: String,
    /// Additional metadata, shared by handle.
    pub meta: Option<SharedMap<String, String>>,
}

/// The auxiliary detail links.
#[derive(Clone, Debug, Default)]
pub struct ExtrasVM {
    /// Links keyed by name, shared by handle.
    pub links: Option<SharedMap<String, LinkVM>>,
}

/// The rendered detail page.
#[derive(Clone, Debug, Default)]
pub struct PageVM {
    /// Resource kind.
    pub kind: crate::definitions::ResourceKind,
    /// Page surface.
    pub mode: ResourceViewMode,
    /// Localized page title.
    pub title: String,
    /// Management breadcrumb.
    pub breadcrumb: BreadcrumbVM,
    /// Selected route names.
    pub routes: ResourceRouteSet,
    /// List target.
    pub back_href: String,
    /// Edit target.
    pub edit_href: String,
    /// Delete target.
    pub delete_action: String,
    /// Delete confirmation message.
    pub delete_confirm: String,
    /// Whether the caller may edit.
    pub can_edit: bool,
    /// Whether the caller is logged in.
    pub is_logged_in: bool,
    /// Whether this is the management surface.
    pub manage_view: bool,
    /// Rendered sections.
    pub sections: Vec<SectionVM>,
    /// Auxiliary links.
    pub extras: ExtrasVM,
    /// The service-produced page SEO value.
    pub seo: SEOVM,
    /// Optional consumer-supplied render props.
    pub seo_view: SEOView,
}
