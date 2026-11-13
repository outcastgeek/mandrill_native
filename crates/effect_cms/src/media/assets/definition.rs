//! The default media resource declaration.
//!
//! These are data declarations only. They register no route, render no page and
//! perform no I/O; later media owners consume the same definitions.

use crate::definitions::{
    DASHBOARD_ACTION_LOCATION_HEADER, DashboardActionLocation, RESOURCE_ACTION_ATTACH,
    RESOURCE_ACTION_CREATE, RESOURCE_ACTION_DELETE, RESOURCE_ACTION_UPDATE, RESOURCE_FIELD_KEY_KIND,
    RESOURCE_FIELD_KEY_STATUS, RESOURCE_FIELD_KEY_TITLE, RESOURCE_FIELD_KEY_URL,
    RESOURCE_FIELD_KIND_SELECT, RESOURCE_FIELD_KIND_TEXT, RESOURCE_FIELD_KIND_TEXT_AREA,
    RESOURCE_FIELD_KIND_URL, RESOURCE_FILTER_KIND, RESOURCE_FILTER_SEARCH, RESOURCE_FILTER_SORT,
    RESOURCE_FILTER_STATUS, RESOURCE_LIST_LAYOUT_RESULTS, RESOURCE_ROUTE_SLOT_NEW,
    RESOURCE_SECTION_KEY_IDENTITY, RESOURCE_SECTION_KEY_METADATA, RESOURCE_SECTION_KEY_RESULTS,
    ResourceAction, ResourceDashboardActionDefinition, ResourceDefinition,
    ResourceDisplayDefinition, ResourceFieldKey, ResourceFieldKind, ResourceFilterDefinition,
    ResourceFilterKey, ResourceFilterOptionDefinition, ResourceFilterValue,
    ResourceFormDefinition, ResourceFormFieldDefinition, ResourceFormSectionDefinition, ResourceKind,
    ResourceListDefinition, ResourceListItemDefinition, ResourceListItemMetaDefinition,
    ResourceListLayout, ResourceListSectionDefinition, ResourceRouteSet, ResourceRouteSlot,
    ResourceSEODefinition, ResourceSectionKey, ResourceSitemapDefinition, SEO_PAGE_TYPE_WEBSITE,
    SEOPageType, SharedList,
};

use super::model::{
    MEDIA_KIND_DOCUMENT, MEDIA_KIND_IMAGE, MEDIA_KIND_VIDEO, MEDIA_SORT_ALPHA, MEDIA_SORT_NEWEST,
    MEDIA_SORT_OLDEST, MEDIA_STATUS_ARCHIVED, MEDIA_STATUS_DRAFT, MEDIA_STATUS_READY,
};

/// Options for the default media resource definition.
#[derive(Clone, Debug, Default)]
pub struct MediaResourceDefinitionOptions {
    /// Resource kind.
    pub kind: ResourceKind,
    /// Management route names.
    pub management_routes: ResourceRouteSet,
    /// Display title keys.
    pub display: ResourceDisplayDefinition,
    /// SEO behavior.
    pub seo: ResourceSEODefinition,
    /// Sitemap behavior.
    pub sitemap: ResourceSitemapDefinition,
}

/// Build the default media resource definition.
#[must_use]
pub fn new_media_resource_definition(
    options: MediaResourceDefinitionOptions,
) -> ResourceDefinition {
    ResourceDefinition {
        kind: options.kind,
        display: media_resource_display(options.display),
        management_routes: options.management_routes,
        filters: SharedList::new(default_media_resource_filters()),
        actions: SharedList::new(vec![
            ResourceAction::new(RESOURCE_ACTION_CREATE),
            ResourceAction::new(RESOURCE_ACTION_UPDATE),
            ResourceAction::new(RESOURCE_ACTION_DELETE),
            ResourceAction::new(RESOURCE_ACTION_ATTACH),
        ]),
        dashboard_actions: SharedList::new(default_media_resource_dashboard_actions()),
        seo: media_resource_seo(options.seo),
        sitemap: options.sitemap,
        list: default_media_resource_list_definition(),
        form: default_media_resource_form_definition(),
        ..ResourceDefinition::default()
    }
}

/// The default media list definition.
#[must_use]
pub fn default_media_resource_list_definition() -> ResourceListDefinition {
    ResourceListDefinition {
        layout: ResourceListLayout::new(RESOURCE_LIST_LAYOUT_RESULTS),
        sections: SharedList::new(vec![ResourceListSectionDefinition {
            key: ResourceSectionKey::new(RESOURCE_SECTION_KEY_RESULTS),
            ..ResourceListSectionDefinition::default()
        }]),
        item: ResourceListItemDefinition {
            meta: SharedList::new(vec![
                ResourceListItemMetaDefinition {
                    label_key: "media.form.kind_label".to_owned(),
                    ..ResourceListItemMetaDefinition::default()
                },
                ResourceListItemMetaDefinition {
                    label_key: "common.status".to_owned(),
                    ..ResourceListItemMetaDefinition::default()
                },
            ]),
        },
    }
}

/// The default media filters.
#[must_use]
pub fn default_media_resource_filters() -> Vec<ResourceFilterDefinition> {
    vec![
        ResourceFilterDefinition {
            key: ResourceFilterKey::new(RESOURCE_FILTER_KIND),
            default_value: ResourceFilterValue::new(""),
            allowed: SharedList::new(vec![
                ResourceFilterValue::new(""),
                ResourceFilterValue::new(MEDIA_KIND_IMAGE),
                ResourceFilterValue::new(MEDIA_KIND_VIDEO),
                ResourceFilterValue::new(MEDIA_KIND_DOCUMENT),
            ]),
            options: SharedList::new(vec![
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(""),
                    label_key: "media.filter.kinds_all".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_KIND_IMAGE),
                    label_key: "media.filter.images".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_KIND_VIDEO),
                    label_key: "media.filter.videos".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_KIND_DOCUMENT),
                    label_key: "media.filter.documents".to_owned(),
                },
            ]),
            ..ResourceFilterDefinition::default()
        },
        ResourceFilterDefinition {
            key: ResourceFilterKey::new(RESOURCE_FILTER_STATUS),
            default_value: ResourceFilterValue::new(""),
            allowed: SharedList::new(vec![
                ResourceFilterValue::new(""),
                ResourceFilterValue::new(MEDIA_STATUS_READY),
                ResourceFilterValue::new(MEDIA_STATUS_DRAFT),
                ResourceFilterValue::new(MEDIA_STATUS_ARCHIVED),
            ]),
            options: SharedList::new(vec![
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(""),
                    label_key: "media.filter.statuses_all".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_STATUS_READY),
                    label_key: "media.status.ready".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_STATUS_DRAFT),
                    label_key: "media.status.draft".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_STATUS_ARCHIVED),
                    label_key: "media.status.archived".to_owned(),
                },
            ]),
            ..ResourceFilterDefinition::default()
        },
        ResourceFilterDefinition {
            key: ResourceFilterKey::new(RESOURCE_FILTER_SORT),
            default_value: ResourceFilterValue::new(MEDIA_SORT_NEWEST),
            allowed: SharedList::new(vec![
                ResourceFilterValue::new(MEDIA_SORT_NEWEST),
                ResourceFilterValue::new(MEDIA_SORT_OLDEST),
                ResourceFilterValue::new(MEDIA_SORT_ALPHA),
            ]),
            options: SharedList::new(vec![
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_SORT_NEWEST),
                    label_key: "media.picker.sort_newest".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_SORT_OLDEST),
                    label_key: "media.picker.sort_oldest".to_owned(),
                },
                ResourceFilterOptionDefinition {
                    value: ResourceFilterValue::new(MEDIA_SORT_ALPHA),
                    label_key: "media.picker.sort_alpha".to_owned(),
                },
            ]),
            ..ResourceFilterDefinition::default()
        },
        ResourceFilterDefinition {
            key: ResourceFilterKey::new(RESOURCE_FILTER_SEARCH),
            ..ResourceFilterDefinition::default()
        },
    ]
}

/// The default media form definition.
#[must_use]
pub fn default_media_resource_form_definition() -> ResourceFormDefinition {
    ResourceFormDefinition {
        sections: SharedList::new(vec![
            form_section(
                RESOURCE_SECTION_KEY_IDENTITY,
                "media.form.source_section_title",
                "media.form.source_section_description",
                vec![
                    form_field(
                        RESOURCE_FIELD_KEY_KIND,
                        RESOURCE_FIELD_KIND_SELECT,
                        "kind",
                        "media.form.kind_label",
                    )
                    .help("media.form.kind_help")
                    .required()
                    .build(),
                    form_field(
                        "source_kind",
                        RESOURCE_FIELD_KIND_SELECT,
                        "source_kind",
                        "media.form.source_kind_label",
                    )
                    .help("media.form.source_kind_help")
                    .required()
                    .build(),
                    form_field(
                        RESOURCE_FIELD_KEY_URL,
                        RESOURCE_FIELD_KIND_TEXT,
                        "url",
                        "media.form.url_label",
                    )
                    .placeholder("media.form.url_placeholder")
                    .help("media.form.url_help")
                    .required()
                    .build(),
                ],
            ),
            form_section(
                RESOURCE_SECTION_KEY_METADATA,
                "media.form.metadata_section_title",
                "media.form.metadata_section_description",
                vec![
                    form_field(
                        RESOURCE_FIELD_KEY_TITLE,
                        RESOURCE_FIELD_KIND_TEXT,
                        "title",
                        "media.form.title_label",
                    )
                    .placeholder("media.form.title_placeholder")
                    .build(),
                    form_field(
                        "description",
                        RESOURCE_FIELD_KIND_TEXT_AREA,
                        "description",
                        "media.form.description_label",
                    )
                    .placeholder("media.form.description_placeholder")
                    .rows(4)
                    .build(),
                    form_field(
                        "alt",
                        RESOURCE_FIELD_KIND_TEXT,
                        "alt",
                        "media.form.alt_label",
                    )
                    .placeholder("media.form.alt_placeholder")
                    .help("media.form.alt_help")
                    .build(),
                    form_field(
                        "thumbnail_url",
                        RESOURCE_FIELD_KIND_URL,
                        "thumbnail_url",
                        "media.form.thumbnail_label",
                    )
                    .placeholder("media.form.thumbnail_placeholder")
                    .help("media.form.thumbnail_help")
                    .build(),
                    form_field(
                        "duration",
                        RESOURCE_FIELD_KIND_TEXT,
                        "duration",
                        "media.form.duration_label",
                    )
                    .placeholder("media.form.duration_placeholder")
                    .help("media.form.duration_help")
                    .build(),
                    form_field(
                        RESOURCE_FIELD_KEY_STATUS,
                        RESOURCE_FIELD_KIND_SELECT,
                        "status",
                        "media.form.status_label",
                    )
                    .help("media.form.status_help")
                    .required()
                    .build(),
                ],
            ),
        ]),
    }
}

/// The default media dashboard actions.
#[must_use]
pub fn default_media_resource_dashboard_actions() -> Vec<ResourceDashboardActionDefinition> {
    vec![ResourceDashboardActionDefinition {
        location: DashboardActionLocation::new(DASHBOARD_ACTION_LOCATION_HEADER),
        label_key: "contentmgmt.media_dashboard.new_media".to_owned(),
        icon: "plus".to_owned(),
        route_slot: ResourceRouteSlot::new(RESOURCE_ROUTE_SLOT_NEW),
        ..ResourceDashboardActionDefinition::default()
    }]
}

fn form_section(
    key: &str,
    title_key: &str,
    description_key: &str,
    fields: Vec<ResourceFormFieldDefinition>,
) -> ResourceFormSectionDefinition {
    ResourceFormSectionDefinition {
        key: ResourceSectionKey::new(key),
        title_key: title_key.to_owned(),
        description_key: description_key.to_owned(),
        fields: SharedList::new(fields),
    }
}

struct FormFieldBuilder(ResourceFormFieldDefinition);

impl FormFieldBuilder {
    fn placeholder(mut self, key: &str) -> Self {
        self.0.placeholder_key = key.to_owned();
        self
    }

    fn help(mut self, key: &str) -> Self {
        self.0.help_text_key = key.to_owned();
        self
    }

    fn rows(mut self, rows: i64) -> Self {
        self.0.rows = rows;
        self
    }

    fn required(mut self) -> Self {
        self.0.required = true;
        self
    }

    fn build(self) -> ResourceFormFieldDefinition {
        self.0
    }
}

fn form_field(key: &str, kind: &str, name: &str, label_key: &str) -> FormFieldBuilder {
    FormFieldBuilder(ResourceFormFieldDefinition {
        key: ResourceFieldKey::new(key),
        kind: ResourceFieldKind::new(kind),
        name: name.to_owned(),
        label_key: label_key.to_owned(),
        ..ResourceFormFieldDefinition::default()
    })
}

fn media_resource_display(mut display: ResourceDisplayDefinition) -> ResourceDisplayDefinition {
    if display.dashboard_title_key.is_empty() {
        display.dashboard_title_key = "contentmgmt.media_dashboard.title".to_owned();
    }
    if display.dashboard_subtitle_key.is_empty() {
        display.dashboard_subtitle_key = "contentmgmt.media_dashboard.subtitle".to_owned();
    }
    if display.list_title_key.is_empty() {
        display.list_title_key = "media.list.title".to_owned();
    }
    if display.list_subtitle_key.is_empty() {
        display.list_subtitle_key = "media.list.subtitle".to_owned();
    }
    if display.new_title_key.is_empty() {
        display.new_title_key = "media.form.new_title".to_owned();
    }
    if display.edit_title_key.is_empty() {
        display.edit_title_key = "media.form.edit_title".to_owned();
    }
    if display.detail_title_key.is_empty() {
        display.detail_title_key = "media.list.title".to_owned();
    }
    display
}

fn media_resource_seo(mut seo: ResourceSEODefinition) -> ResourceSEODefinition {
    if seo.page_type.as_str().is_empty() {
        seo.page_type = SEOPageType::new(SEO_PAGE_TYPE_WEBSITE);
    }
    if seo.list_title_key.is_empty() {
        seo.list_title_key = "media.list.page_title".to_owned();
    }
    if seo.list_description_key.is_empty() {
        seo.list_description_key = "media.list.subtitle".to_owned();
    }
    seo
}
