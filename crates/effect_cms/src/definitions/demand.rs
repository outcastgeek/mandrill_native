//! Checked declaration payload demand before startup normalization.
use std::mem::{size_of, size_of_val};

use effect::memory::{Bytes, MemoryError};

use super::builders::resource_uses_unified_management_routes;
use super::model::{ResourceDefinition, ResourceRouteSet};
use super::relations::{ResourceRelationDefinition, ResourceRelationKeyDefinition};

pub(super) struct DefinitionDemand {
    used: usize,
    maximum: usize,
}

impl DefinitionDemand {
    pub(super) fn new(maximum: usize) -> Self {
        Self { used: 0, maximum }
    }

    pub(super) fn account(&mut self, value: &ResourceDefinition) -> Result<(), MemoryError> {
        let display = &value.display;
        let seo = &value.seo;
        self.text([
            value.kind.as_str(),
            value.management_route_mode.as_str(),
            display.dashboard_title_key.as_str(),
            display.dashboard_subtitle_key.as_str(),
            display.list_title_key.as_str(),
            display.list_subtitle_key.as_str(),
            display.new_title_key.as_str(),
            display.edit_title_key.as_str(),
            display.detail_title_key.as_str(),
            seo.page_type.as_str(),
            seo.list_title_key.as_str(),
            seo.list_description_key.as_str(),
            value.sitemap.route_name.as_str(),
            value.sitemap.change_frequency.as_str(),
            value.list.layout.as_str(),
        ])?;
        self.routes(&value.public_routes)?;
        self.routes(&value.management_routes)?;
        if resource_uses_unified_management_routes(value) {
            let routes = &value.management_routes;
            let normalized = if routes.dashboard.trim().is_empty() {
                routes.list.trim()
            } else {
                routes.dashboard.trim()
            };
            // Preparation may briefly hold both the original and replacement list name.
            self.add(normalized.len())?;
        }

        value.filters.with(|items| {
            self.list(items, |meter, filter| {
                meter.text([
                    filter.key.as_str(),
                    filter.param.as_str(),
                    filter.mode.as_str(),
                    filter.label_key.as_str(),
                    filter.add_label_key.as_str(),
                    filter.remove_label_key.as_str(),
                    filter.default_value.as_str(),
                ])?;
                filter.allowed.with(|values| {
                    meter.list(values, |meter, value| meter.add(value.as_str().len()))
                })?;
                filter.options.with(|options| {
                    meter.list(options, |meter, option| {
                        meter.text([option.value.as_str(), option.label_key.as_str()])
                    })
                })
            })
        })?;
        value
            .actions
            .with(|items| self.list(items, |meter, action| meter.add(action.as_str().len())))?;
        value.widgets.with(|items| {
            self.list(items, |meter, widget| {
                meter.text([widget.widget_key.as_str(), widget.scope.as_str()])
            })
        })?;
        value.dashboard_actions.with(|items| {
            self.list(items, |meter, action| {
                meter.text([
                    action.location.as_str(),
                    action.label_key.as_str(),
                    action.icon.as_str(),
                    action.route_slot.as_str(),
                    action.route_name.as_str(),
                ])?;
                action.filters.with(|filters| {
                    meter.list(filters, |meter, filter| {
                        meter.text([filter.key.as_str(), filter.value.as_str()])
                    })
                })
            })
        })?;

        value.list.sections.with(|items| {
            self.list(items, |meter, section| {
                meter.text([
                    section.key.as_str(),
                    section.title_key.as_str(),
                    section.description_key.as_str(),
                ])
            })
        })?;
        value.list.item.meta.with(|items| {
            self.list(items, |meter, field| {
                meter.text([field.label.as_str(), field.label_key.as_str()])
            })
        })?;
        value.dashboard.sections.with(|items| {
            self.list(items, |meter, section| {
                meter.text([
                    section.key.as_str(),
                    section.title_key.as_str(),
                    section.description_key.as_str(),
                ])
            })
        })?;
        value.form.sections.with(|items| {
            self.list(items, |meter, section| {
                meter.text([
                    section.key.as_str(),
                    section.title_key.as_str(),
                    section.description_key.as_str(),
                ])?;
                section.fields.with(|fields| {
                    meter.list(fields, |meter, field| {
                        meter.text([
                            field.key.as_str(),
                            field.kind.as_str(),
                            field.name.as_str(),
                            field.input_id.as_str(),
                            field.label_key.as_str(),
                            field.placeholder_key.as_str(),
                            field.help_text_key.as_str(),
                            field.description_key.as_str(),
                            field.secondary_name.as_str(),
                            field.secondary_id.as_str(),
                        ])
                    })
                })
            })
        })?;
        value.detail.sections.with(|items| {
            self.list(items, |meter, section| {
                meter.text([
                    section.key.as_str(),
                    section.title_key.as_str(),
                    section.description_key.as_str(),
                ])?;
                section.fields.with(|fields| {
                    meter.list(fields, |meter, field| {
                        meter.text([
                            field.key.as_str(),
                            field.name.as_str(),
                            field.label_key.as_str(),
                        ])
                    })
                })
            })
        })?;
        self.list(&value.relations, Self::relation)
    }

    fn routes(&mut self, routes: &ResourceRouteSet) -> Result<(), MemoryError> {
        self.text([
            routes.dashboard.as_str(),
            routes.list.as_str(),
            routes.detail.as_str(),
            routes.new.as_str(),
            routes.new_post.as_str(),
            routes.edit.as_str(),
            routes.edit_post.as_str(),
            routes.delete.as_str(),
        ])?;
        routes.aux.with(|values| {
            for (key, value) in values {
                self.add(size_of::<(super::vocabulary::ResourceAuxRoute, String)>())?;
                self.text([key.as_str(), value.as_str()])?;
            }
            Ok(())
        })
    }

    fn relation(&mut self, relation: &ResourceRelationDefinition) -> Result<(), MemoryError> {
        let editor = &relation.editor;
        self.text([
            relation.name.as_str(),
            relation.cardinality.as_str(),
            relation.inverse_of.as_str(),
            relation.on_target_delete.as_str(),
            relation.query_binding.as_str(),
            relation.scope.as_str(),
            relation.target.as_str(),
            relation.discriminator.as_str(),
            editor.control.as_str(),
            editor.label_query.as_str(),
            editor.locale_key.as_str(),
            editor.save_mode.as_str(),
            editor.removal_operation.as_str(),
            editor.label_field.as_str(),
            editor.order_field.as_str(),
        ])?;
        self.list(&relation.targets, |meter, target| {
            meter.text([
                target.scope.as_str(),
                target.tag.as_str(),
                target.target.as_str(),
            ])?;
            meter.list(&target.keys, Self::relation_key)
        })?;
        self.list(&relation.keys, Self::relation_key)?;
        self.list(&relation.through, |meter, text| meter.add(text.len()))?;
        self.list(&relation.operations, |meter, text| meter.add(text.len()))?;
        self.list(&editor.columns, |meter, column| {
            meter.add(column.as_str().len())
        })
    }

    fn relation_key(&mut self, key: &ResourceRelationKeyDefinition) -> Result<(), MemoryError> {
        self.text([
            key.source.as_str(),
            key.target.as_str(),
            key.r#type.as_str(),
        ])
    }

    fn list<T>(
        &mut self,
        values: &[T],
        mut account: impl FnMut(&mut Self, &T) -> Result<(), MemoryError>,
    ) -> Result<(), MemoryError> {
        self.add(size_of_val(values))?;
        for value in values {
            account(self, value)?;
        }
        Ok(())
    }

    fn text<const N: usize>(&mut self, fields: [&str; N]) -> Result<(), MemoryError> {
        for field in fields {
            self.add(field.len())?;
        }
        Ok(())
    }

    fn add(&mut self, amount: usize) -> Result<(), MemoryError> {
        self.used = self
            .used
            .checked_add(amount)
            .ok_or(MemoryError::ArithmeticOverflow)?;
        if self.used > self.maximum {
            return Err(MemoryError::ValueTooLarge {
                requested: Bytes::new(self.used),
                capacity: Bytes::new(self.maximum),
            });
        }
        Ok(())
    }
}
