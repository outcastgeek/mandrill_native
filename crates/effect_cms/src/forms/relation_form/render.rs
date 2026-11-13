//! Maud rendering for the relation control.
//!
//! The control renders only a valid state: the fieldset and its legend, the
//! localized help and required marker, the optional search input and action, the
//! related-row editor or the selection control with its rejected-selection transport
//! and selected links, the declared action buttons, the issue list, the status line
//! and the pagination links.
//!
//! Every identity and relationship is the state's: the control id, the search id,
//! the status id, the error-list id, the search action id, the previous/next ids, the
//! row ids and the tab/error suffixes. Client attributes from the caller's maps are
//! retained through the canonical shared controls and Maud attribute spreading.

use maud::{Markup, html};

use web_components::components::shared;
use web_components::components::unstyled::button::{ButtonProps,Button};
use web_components::components::unstyled::input::{INPUT_TYPE_HIDDEN, INPUT_TYPE_TEXT, InputProps,Input,InputWithLabel};
use web_components::components::unstyled::select::{SelectOption, SelectProps,Select};

use super::super::localized_tabs::localized_submit_control_attributes;
use super::{RelationFormActionInput, RelationFormState, relation_form_description_ids};

/// Render the relation control.
#[must_use]
pub fn relation_control(state: &RelationFormState) -> Markup {
    if !state.valid() {
        return Markup::default();
    }
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    let required = super::relation_form_required(&value.descriptor);
    html! {
        fieldset id=(value.id) class="min-w-0 space-y-4" aria-describedby=(relation_form_description_ids(state)) {
            legend class="text-sm font-medium" {
                (value.labels.label)
                @if required {
                    span { (value.labels.required) }
                }
            }
            @if !value.labels.help.is_empty() {
                p id={ (value.id) "-help" } class="text-sm text-muted-foreground" { (value.labels.help) }
            }
            @if value.searchable {
                div class="flex flex-wrap items-end gap-2" {
                    div class="min-w-0 flex-1" {
                        (relation_search_input(state))
                    }
                    (relation_search_action(state))
                }
            }
            @if value.descriptor.definition().map(|definition| definition.editor.control).unwrap_or_default() == "related_rows" {
                (relation_rows_control(state))
            } @else {
                (relation_select(state))
                @for draft in &value.rejected_selections {
                    (relation_rejected_selection(state, draft))
                }
                @for record in state.lookup_selected() {
                    @if !record.href().is_empty() {
                        @let link_props = ButtonProps {
                            as_element: shared::ELEMENT_A,
                            href: record.href(),
                            variant: shared::VARIANT_OUTLINE,
                            ..ButtonProps::default()
                        };
                        (Button(&link_props,html!{(record.label())}))
                    }
                }
            }
            @for action in &value.actions {
                (relation_operation_button(action, value.labels.label.as_str()))
            }
            @if !value.issues.is_empty() {
                ul id={ (value.id) "-errors" } class="space-y-1 text-sm text-destructive" {
                    @for issue in &value.issues {
                        li { (issue.message) }
                    }
                }
            }
            p id={ (value.id) "-status" } role="status" class="text-sm text-muted-foreground" {
                @if value.options.is_empty() && value.rows.is_empty() {
                    (value.labels.empty)
                } @else {
                    (value.labels.status)
                }
            }
            @if !value.previous_href.is_empty() || !value.next_href.is_empty() {
                nav aria-label=(value.labels.pagination) class="flex flex-wrap gap-3" {
                    @if !value.previous_href.is_empty() {
                        @let previous_props = ButtonProps {
                            as_element: shared::ELEMENT_A,
                            href: value.previous_href.clone(),
                            variant: shared::VARIANT_OUTLINE,
                            attributes:relation_navigation_attributes(&value.previous_attributes,&value.id,"previous"),
                            ..ButtonProps::default()
                        };
                        (Button(&previous_props,html!{(value.previous_label)}))
                    }
                    @if !value.next_href.is_empty() {
                        @let next_props = ButtonProps {
                            as_element: shared::ELEMENT_A,
                            href: value.next_href.clone(),
                            variant: shared::VARIANT_OUTLINE,
                            attributes:relation_navigation_attributes(&value.next_attributes,&value.id,"next"),
                            ..ButtonProps::default()
                        };
                        (Button(&next_props,html!{(value.next_label)}))
                    }
                }
            }
        }
    }
}

fn relation_search_input(state: &RelationFormState) -> Markup {
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    let props = InputProps {
        input_type: INPUT_TYPE_TEXT,
        name: format!("{}.search", value.name),
        id: format!("{}-search", value.id),
        value: crate::byte_text::encoding_text(state.query().search().as_bytes()),
        full_width: true,
        ..InputProps::default()
    };
    let mut attributes = value.search_attributes.clone();
    attributes.insert(
        "data-on:keydown".to_owned(),
        format!(
            "if (evt.key === 'Enter' && !evt.isComposing) {{ evt.preventDefault(); document.getElementById({})?.click(); }}",
            serde_json::to_string(format!("{}-search-action", value.id).as_str())
                .unwrap_or_else(|_| "\"\"".to_owned())
        ),
    );
    let props=InputProps{attributes,..props};
    InputWithLabel(&props,&value.labels.search,"")
}

fn relation_search_action(state: &RelationFormState) -> Markup {
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    let mut attributes=value.search_action_attributes.clone();attributes.insert("id".into(),format!("{}-search-action",value.id));
    Button(&ButtonProps{variant:shared::VARIANT_OUTLINE,button_type:"button",attributes,..Default::default()},html!{(&value.labels.search_action)})
}

fn relation_select(state: &RelationFormState) -> Markup {
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    let definition = value.descriptor.definition().unwrap_or_default();
    let multiple = definition.cardinality == "one_to_many" || definition.cardinality == "many_to_many";
    let mut options = Vec::new();
    let mut selected_value = String::new();
    if !multiple {
        options.push(SelectOption {
            label: value.labels.choose.clone(),
            value: String::new(),
            ..SelectOption::default()
        });
    }
    for option in &value.options {
        if !multiple && option.selected {
            selected_value = option.value.clone();
        }
        options.push(SelectOption {
            label: option.label.clone(),
            value: option.value.clone(),
            disabled: false,
            selected: option.selected,
            attributes: Default::default(),
        });
    }
    let props = SelectProps {
        id: format!("{}-selection", value.id),
        name: value.name.clone(),
        required: super::relation_form_required(&value.descriptor),
        value: selected_value,
        options,
        class: "[&[multiple]]:h-auto".to_owned(),
        aria_label: value.labels.label.clone(),
        aria_described_by: relation_form_description_ids(state),
        ..SelectProps::default()
    };
    let mut attributes=std::collections::BTreeMap::new();
    attributes.insert("aria-invalid".into(),(!value.issues.is_empty()).to_string());
    if multiple{attributes.insert("multiple".into(),"multiple".into());}
    Select(&SelectProps{attributes,..props})
}

fn relation_rejected_selection(
    state: &RelationFormState,
    draft: &super::RelationRejectedSelectionDraft,
) -> Markup {
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    let key_input = InputProps {
        input_type: INPUT_TYPE_HIDDEN,
        name: format!("{}.rejected_keys", value.name),
        value: draft.key.clone(),
        ..InputProps::default()
    };
    let raw_input = InputProps {
        input_type: INPUT_TYPE_HIDDEN,
        name: format!("{}.rejected", value.name),
        value: draft.raw.clone(),
        ..InputProps::default()
    };
    html! {
        div id={ (value.id) "-rejected-" (draft.key) } class="space-y-2 rounded-md border border-destructive p-3" {
            (Input(&InputProps{attributes:[("data-relation-rejected-transport".into(),"true".into())].into(),..key_input}))
            (Input(&InputProps{attributes:[("data-relation-rejected-transport".into(),"true".into())].into(),..raw_input}))
            p class="text-sm text-destructive" { (value.labels.unavailable) }
            code class="block break-all text-sm" { (draft.raw) }
            (relation_remove_rejected_button(state, draft.key.as_str()))
        }
    }
}

fn relation_remove_rejected_button(state: &RelationFormState, key: &str) -> Markup {
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    let mut attributes = std::collections::BTreeMap::new();
    attributes.insert("name".to_owned(), format!("{}.rejected_remove", value.name));
    attributes.insert("value".to_owned(), key.to_owned());
    attributes.insert("formnovalidate".to_owned(), "true".to_owned());
    let props = ButtonProps {
        variant: shared::VARIANT_OUTLINE,
        button_type: "submit",
        ..ButtonProps::default()
    };
    let submit_attributes = localized_submit_control_attributes(attributes);
    Button(&ButtonProps{attributes:submit_attributes,..props},html!{(&value.labels.remove_rejected)})
}

fn relation_rows_control(state: &RelationFormState) -> Markup {
    let Some(value) = state.value.as_ref() else {
        return Markup::default();
    };
    html! {
        div class="space-y-4" {
            @for row in &value.rows {
                fieldset id=(row.id()) class="min-w-0 space-y-3 rounded-md border border-border p-4" {
                    legend class="px-1 text-sm font-medium" { (row.label()) }
                    @if let Some(render_row) = value.render_row.as_ref() {
                        (render_row(row))
                    }
                    @if !row.issues().is_empty() {
                        ul id={ (row.id()) "-errors" } class="space-y-1 text-sm text-destructive" {
                            @for issue in row.issues() {
                                li { (issue.message) }
                            }
                        }
                    }
                    div class="flex flex-wrap gap-2" {
                        @for action in row.actions() {
                            (relation_operation_button_ref(&action, row.label().as_str()))
                        }
                    }
                }
            }
        }
    }
}

fn relation_operation_button(action: &RelationFormActionInput, context_label: &str) -> Markup {
    relation_operation_button_ref(action, context_label)
}

fn relation_operation_button_ref(action: &RelationFormActionInput, context_label: &str) -> Markup {
    let variant = if action.operation == "delete_related" {
        shared::VARIANT_DESTRUCTIVE
    } else {
        shared::VARIANT_OUTLINE
    };
    let props = ButtonProps {
        variant,
        button_type: "submit",
        aria_label: format!("{} — {context_label}", action.label),
        ..ButtonProps::default()
    };
    let mut attributes = std::collections::BTreeMap::new();
    attributes.insert("name".to_owned(), action.name.clone());
    attributes.insert("value".to_owned(), action.value.clone());
    let submit_attributes = localized_submit_control_attributes(attributes);
    Button(&ButtonProps{attributes:submit_attributes,..props},html!{(&action.label)})
}


fn relation_navigation_attributes(values:&std::collections::BTreeMap<String,String>,id:&str,direction:&str)->std::collections::BTreeMap<String,String>{
    let mut attributes=values.clone();attributes.insert("id".into(),format!("{id}-{direction}"));attributes
}
