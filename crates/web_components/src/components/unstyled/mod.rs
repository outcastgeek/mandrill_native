pub mod accordion;
pub mod accountmenu;
pub mod activityfeed;
pub mod aiicon;
pub mod alert;
pub mod analytics;
pub mod aspectratio;
pub mod autocomplete;
pub mod avatar;
pub mod backlink;
pub mod badge;
pub mod blockquote;
pub mod bottomnav;
pub mod breadcrumb;
pub mod button;
pub mod card;
pub mod carousel;
pub mod chatinput;
pub mod chatmessage;
pub mod checkbox;
pub mod checkmarkitem;
pub mod checkmarklist;
pub mod circularprogress;
pub mod code;
pub mod collapsible;
pub mod command;
pub mod companylogo;
pub mod companylogosgrid;
pub mod confirmdialog;
pub mod container;
pub mod copybutton;
pub mod dashboard;
pub mod datepicker;
pub mod datetimepicker;
pub mod democard;
pub mod dialog;
pub mod divider;
pub mod document;
pub mod drawer;
pub mod durationbadge;
pub mod durationpicker;
pub mod emptystate;
pub mod featuregrid;
pub mod field;
pub mod form;
pub mod formerror;
pub mod formfield;
pub mod gamification;
pub mod gradienttext;
pub mod grid;
pub mod heading;
pub mod iconbox;
pub mod icons;
pub mod image;
pub mod indicators;
pub mod input;
pub mod label;
pub mod languageselector;
pub mod learning;
pub mod list;
pub mod megadropdown;
pub mod megamenufooter;
pub mod menu;
pub mod mermaid;
pub mod minimalfeaturelink;
pub mod nav;
pub mod navigationarrow;
pub mod pagination;
pub mod paginationdots;
pub mod playbutton;
pub mod popover;
pub mod producticoncard;
pub mod producticongrid;
pub mod productlink;
pub mod progress;
pub mod radio;
pub mod resourceui;
pub mod scrollarea;
pub mod select;
pub mod selectfield;
pub mod separator;
pub mod sidebar;
pub mod skeleton;
pub mod slidepager;
pub mod slider;
pub mod stack;
pub mod starrating;
pub mod statcard;
pub mod statsbar;
pub mod stepper;
pub mod svg;
pub mod switch;
pub mod switchfield;
pub mod table;
pub mod tabs;
pub mod testimonialcard;
pub mod testimonialcarousel;
pub mod text;
pub mod textarea;
pub mod themetoggle;
pub mod timeago;
pub mod toast;
pub mod tooltip;
pub mod topnav;
pub mod uploadarea;
pub mod videocontainer;
pub mod videoplayer;

use crate::components::ComponentInfo;

#[must_use]
pub fn all_components() -> Vec<ComponentInfo> {
    let mut components = Vec::new();
    components.extend_from_slice(accordion::COMPONENTS);
    components.extend_from_slice(accountmenu::COMPONENTS);
    components.extend_from_slice(activityfeed::COMPONENTS);
    components.extend_from_slice(aiicon::COMPONENTS);
    components.extend_from_slice(alert::COMPONENTS);
    components.extend_from_slice(analytics::COMPONENTS);
    components.extend_from_slice(aspectratio::COMPONENTS);
    components.extend_from_slice(autocomplete::COMPONENTS);
    components.extend_from_slice(avatar::COMPONENTS);
    components.extend_from_slice(backlink::COMPONENTS);
    components.extend_from_slice(badge::COMPONENTS);
    components.extend_from_slice(blockquote::COMPONENTS);
    components.extend_from_slice(bottomnav::COMPONENTS);
    components.extend_from_slice(breadcrumb::COMPONENTS);
    components.extend_from_slice(button::COMPONENTS);
    components.extend_from_slice(card::COMPONENTS);
    components.extend_from_slice(carousel::COMPONENTS);
    components.extend_from_slice(chatinput::COMPONENTS);
    components.extend_from_slice(chatmessage::COMPONENTS);
    components.extend_from_slice(checkbox::COMPONENTS);
    components.extend_from_slice(checkmarkitem::COMPONENTS);
    components.extend_from_slice(checkmarklist::COMPONENTS);
    components.extend_from_slice(circularprogress::COMPONENTS);
    components.extend_from_slice(code::COMPONENTS);
    components.extend_from_slice(collapsible::COMPONENTS);
    components.extend_from_slice(command::COMPONENTS);
    components.extend_from_slice(companylogo::COMPONENTS);
    components.extend_from_slice(companylogosgrid::COMPONENTS);
    components.extend_from_slice(confirmdialog::COMPONENTS);
    components.extend_from_slice(container::COMPONENTS);
    components.extend_from_slice(copybutton::COMPONENTS);
    components.extend_from_slice(dashboard::COMPONENTS);
    components.extend_from_slice(datepicker::COMPONENTS);
    components.extend_from_slice(datetimepicker::COMPONENTS);
    components.extend_from_slice(democard::COMPONENTS);
    components.extend_from_slice(dialog::COMPONENTS);
    components.extend_from_slice(divider::COMPONENTS);
    components.extend_from_slice(document::COMPONENTS);
    components.extend_from_slice(drawer::COMPONENTS);
    components.extend_from_slice(durationbadge::COMPONENTS);
    components.extend_from_slice(durationpicker::COMPONENTS);
    components.extend_from_slice(emptystate::COMPONENTS);
    components.extend_from_slice(featuregrid::COMPONENTS);
    components.extend_from_slice(field::COMPONENTS);
    components.extend_from_slice(form::COMPONENTS);
    components.extend_from_slice(formerror::COMPONENTS);
    components.extend_from_slice(formfield::COMPONENTS);
    components.extend_from_slice(gamification::COMPONENTS);
    components.extend_from_slice(gradienttext::COMPONENTS);
    components.extend_from_slice(grid::COMPONENTS);
    components.extend_from_slice(heading::COMPONENTS);
    components.extend_from_slice(iconbox::COMPONENTS);
    components.extend_from_slice(icons::COMPONENTS);
    components.extend_from_slice(image::COMPONENTS);
    components.extend_from_slice(indicators::COMPONENTS);
    components.extend_from_slice(input::COMPONENTS);
    components.extend_from_slice(label::COMPONENTS);
    components.extend_from_slice(languageselector::COMPONENTS);
    components.extend_from_slice(learning::COMPONENTS);
    components.extend_from_slice(list::COMPONENTS);
    components.extend_from_slice(megadropdown::COMPONENTS);
    components.extend_from_slice(megamenufooter::COMPONENTS);
    components.extend_from_slice(menu::COMPONENTS);
    components.extend_from_slice(mermaid::COMPONENTS);
    components.extend_from_slice(minimalfeaturelink::COMPONENTS);
    components.extend_from_slice(nav::COMPONENTS);
    components.extend_from_slice(navigationarrow::COMPONENTS);
    components.extend_from_slice(pagination::COMPONENTS);
    components.extend_from_slice(paginationdots::COMPONENTS);
    components.extend_from_slice(playbutton::COMPONENTS);
    components.extend_from_slice(popover::COMPONENTS);
    components.extend_from_slice(producticoncard::COMPONENTS);
    components.extend_from_slice(producticongrid::COMPONENTS);
    components.extend_from_slice(productlink::COMPONENTS);
    components.extend_from_slice(progress::COMPONENTS);
    components.extend_from_slice(radio::COMPONENTS);
    components.extend_from_slice(resourceui::COMPONENTS);
    components.extend_from_slice(scrollarea::COMPONENTS);
    components.extend_from_slice(select::COMPONENTS);
    components.extend_from_slice(selectfield::COMPONENTS);
    components.extend_from_slice(separator::COMPONENTS);
    components.extend_from_slice(sidebar::COMPONENTS);
    components.extend_from_slice(skeleton::COMPONENTS);
    components.extend_from_slice(slidepager::COMPONENTS);
    components.extend_from_slice(slider::COMPONENTS);
    components.extend_from_slice(stack::COMPONENTS);
    components.extend_from_slice(starrating::COMPONENTS);
    components.extend_from_slice(statcard::COMPONENTS);
    components.extend_from_slice(statsbar::COMPONENTS);
    components.extend_from_slice(stepper::COMPONENTS);
    components.extend_from_slice(switch::COMPONENTS);
    components.extend_from_slice(switchfield::COMPONENTS);
    components.extend_from_slice(svg::COMPONENTS);
    components.extend_from_slice(table::COMPONENTS);
    components.extend_from_slice(tabs::COMPONENTS);
    components.extend_from_slice(testimonialcard::COMPONENTS);
    components.extend_from_slice(testimonialcarousel::COMPONENTS);
    components.extend_from_slice(text::COMPONENTS);
    components.extend_from_slice(textarea::COMPONENTS);
    components.extend_from_slice(themetoggle::COMPONENTS);
    components.extend_from_slice(timeago::COMPONENTS);
    components.extend_from_slice(toast::COMPONENTS);
    components.extend_from_slice(tooltip::COMPONENTS);
    components.extend_from_slice(topnav::COMPONENTS);
    components.extend_from_slice(uploadarea::COMPONENTS);
    components.extend_from_slice(videocontainer::COMPONENTS);
    components.extend_from_slice(videoplayer::COMPONENTS);
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::shared;
    use maud::html;

    #[test]
    fn all_components_includes_every_unstyled_module_registry() {
        let components = all_components();
        let expected_len = accordion::COMPONENTS.len()
            + accountmenu::COMPONENTS.len()
            + activityfeed::COMPONENTS.len()
            + aiicon::COMPONENTS.len()
            + alert::COMPONENTS.len()
            + analytics::COMPONENTS.len()
            + aspectratio::COMPONENTS.len()
            + autocomplete::COMPONENTS.len()
            + avatar::COMPONENTS.len()
            + backlink::COMPONENTS.len()
            + badge::COMPONENTS.len()
            + blockquote::COMPONENTS.len()
            + bottomnav::COMPONENTS.len()
            + breadcrumb::COMPONENTS.len()
            + button::COMPONENTS.len()
            + card::COMPONENTS.len()
            + carousel::COMPONENTS.len()
            + chatinput::COMPONENTS.len()
            + chatmessage::COMPONENTS.len()
            + checkbox::COMPONENTS.len()
            + checkmarkitem::COMPONENTS.len()
            + checkmarklist::COMPONENTS.len()
            + circularprogress::COMPONENTS.len()
            + code::COMPONENTS.len()
            + collapsible::COMPONENTS.len()
            + command::COMPONENTS.len()
            + companylogo::COMPONENTS.len()
            + companylogosgrid::COMPONENTS.len()
            + confirmdialog::COMPONENTS.len()
            + container::COMPONENTS.len()
            + copybutton::COMPONENTS.len()
            + dashboard::COMPONENTS.len()
            + datepicker::COMPONENTS.len()
            + datetimepicker::COMPONENTS.len()
            + democard::COMPONENTS.len()
            + dialog::COMPONENTS.len()
            + divider::COMPONENTS.len()
            + document::COMPONENTS.len()
            + drawer::COMPONENTS.len()
            + durationbadge::COMPONENTS.len()
            + durationpicker::COMPONENTS.len()
            + emptystate::COMPONENTS.len()
            + featuregrid::COMPONENTS.len()
            + field::COMPONENTS.len()
            + form::COMPONENTS.len()
            + formerror::COMPONENTS.len()
            + formfield::COMPONENTS.len()
            + gamification::COMPONENTS.len()
            + gradienttext::COMPONENTS.len()
            + grid::COMPONENTS.len()
            + heading::COMPONENTS.len()
            + iconbox::COMPONENTS.len()
            + icons::COMPONENTS.len()
            + image::COMPONENTS.len()
            + indicators::COMPONENTS.len()
            + input::COMPONENTS.len()
            + label::COMPONENTS.len()
            + languageselector::COMPONENTS.len()
            + learning::COMPONENTS.len()
            + list::COMPONENTS.len()
            + megadropdown::COMPONENTS.len()
            + megamenufooter::COMPONENTS.len()
            + menu::COMPONENTS.len()
            + mermaid::COMPONENTS.len()
            + minimalfeaturelink::COMPONENTS.len()
            + nav::COMPONENTS.len()
            + navigationarrow::COMPONENTS.len()
            + pagination::COMPONENTS.len()
            + paginationdots::COMPONENTS.len()
            + playbutton::COMPONENTS.len()
            + popover::COMPONENTS.len()
            + producticoncard::COMPONENTS.len()
            + producticongrid::COMPONENTS.len()
            + productlink::COMPONENTS.len()
            + progress::COMPONENTS.len()
            + radio::COMPONENTS.len()
            + scrollarea::COMPONENTS.len()
            + select::COMPONENTS.len()
            + selectfield::COMPONENTS.len()
            + separator::COMPONENTS.len()
            + sidebar::COMPONENTS.len()
            + skeleton::COMPONENTS.len()
            + slidepager::COMPONENTS.len()
            + slider::COMPONENTS.len()
            + stack::COMPONENTS.len()
            + starrating::COMPONENTS.len()
            + statcard::COMPONENTS.len()
            + statsbar::COMPONENTS.len()
            + stepper::COMPONENTS.len()
            + switch::COMPONENTS.len()
            + switchfield::COMPONENTS.len()
            + svg::COMPONENTS.len()
            + table::COMPONENTS.len()
            + tabs::COMPONENTS.len()
            + testimonialcard::COMPONENTS.len()
            + testimonialcarousel::COMPONENTS.len()
            + text::COMPONENTS.len()
            + textarea::COMPONENTS.len()
            + themetoggle::COMPONENTS.len()
            + timeago::COMPONENTS.len()
            + toast::COMPONENTS.len()
            + tooltip::COMPONENTS.len()
            + topnav::COMPONENTS.len()
            + uploadarea::COMPONENTS.len()
            + videocontainer::COMPONENTS.len()
            + videoplayer::COMPONENTS.len();

        assert_eq!(components.len(), expected_len);
        assert!(
            components
                .iter()
                .all(|component| component.family == "unstyled")
        );
        assert!(
            components
                .iter()
                .any(|component| component.module == "icons")
        );
        assert!(
            components
                .iter()
                .any(|component| component.name == "DatePicker")
        );
        assert!(
            components
                .iter()
                .any(|component| component.name == "DurationPicker")
        );
    }

    #[test]
    fn public_branch_variations_render_real_html_and_helper_outputs() {
        let alert_html = alert::Alert(
            &alert::AlertProps {
                aria_live: "",
                role: "",
                ..alert::AlertProps::default()
            },
            html! { "Body" },
        )
        .into_string();
        assert!(alert_html.contains(">Body</div>"));
        assert!(!alert_html.contains("aria-live"));
        assert!(!alert_html.contains("role=\"alert\""));

        let chat_input_html =
            chatinput::ChatInput(&chatinput::ChatInputProps::default()).into_string();
        assert!(chat_input_html.contains("<textarea"));
        assert!(!chat_input_html.contains("text-[11px] text-muted-foreground"));

        let mermaid = "<pre><code class=\"language-mermaid\">graph TD</code></pre>";
        assert!(!chatmessage::getRichContentClasses(true, mermaid).contains("w-full max-w-none"));

        let code_html = code::Code(
            &code::CodeProps::default().with_inline(false),
            html! { "let x = 1;" },
        )
        .into_string();
        assert!(code_html.contains("<code"));
        assert!(!code_html.contains("language-"));

        let date_html = datepicker::DatePicker(&datepicker::DatePickerProps {
            full_width: true,
            hide_today: true,
            hide_clear: true,
            ..datepicker::DatePickerProps::default()
        })
        .into_string();
        assert!(date_html.contains("w-full"));
        assert!(!date_html.contains(">Today<"));
        assert!(!date_html.contains(">Clear<"));

        let datetime_html = datetimepicker::DateTimePicker(&datetimepicker::DateTimePickerProps {
            full_width: true,
            hide_today: true,
            hide_clear: true,
            ..datetimepicker::DateTimePickerProps::default()
        })
        .into_string();
        assert!(datetime_html.contains("w-full"));
        assert!(!datetime_html.contains(">Today<"));
        assert!(!datetime_html.contains(">Clear<"));

        let dialog_attrs = dialog::DialogProps {
            role: String::new(),
            ..dialog::DialogProps::default()
        }
        .build_attributes();
        assert!(!dialog_attrs.contains_key("role"));
        let trigger_attrs = dialog::DialogTriggerProps {
            type_attr: String::new(),
            ..dialog::DialogTriggerProps::default()
        }
        .build_trigger_attributes();
        assert!(!trigger_attrs.contains_key("type"));
        let content_attrs = dialog::DialogContentProps {
            role: String::new(),
            ..dialog::DialogContentProps::default()
        }
        .build_content_attributes();
        assert!(!content_attrs.contains_key("role"));

        let drawer_trigger_attrs = drawer::triggerButtonAttributes("Toggle drawer", "");
        assert_eq!(
            drawer_trigger_attrs.get("aria-label").map(String::as_str),
            Some("Toggle drawer")
        );
        assert!(!drawer_trigger_attrs.contains_key("data-on:click"));

        assert!(
            durationpicker::DurationPickerProps {
                full_width: true,
                ..durationpicker::DurationPickerProps::default()
            }
            .build_wrapper_classes()
            .contains("w-full")
        );

        let empty_button_html = emptystate::EmptyState(
            &emptystate::EmptyStateProps::default()
                .with_title("Nothing")
                .with_action_on_click("Retry", ""),
            html! {},
        )
        .into_string();
        assert!(empty_button_html.contains("<button"));
        assert!(!empty_button_html.contains("onclick="));

        let field_html = field::Field(
            &field::FieldProps::default()
                .with_helper_text("Help")
                .with_error(true, "Bad"),
            html! { input; },
        )
        .into_string();
        assert!(field_html.contains(">Bad</p>"));
        assert!(!field_html.contains("-helper"));
        assert!(!field_html.contains("-error"));

        let form_error_attrs = formerror::FormErrorProps {
            aria_live: "",
            ..formerror::FormErrorProps::default()
        }
        .build_attributes();
        assert_eq!(
            form_error_attrs.get("role").map(String::as_str),
            Some("alert")
        );
        assert!(!form_error_attrs.contains_key("aria-live"));

        let form_field_html = formfield::FormField(
            &formfield::FormFieldProps::default()
                .with_helper_text("Help")
                .with_error("Bad"),
            html! { input; },
        )
        .into_string();
        assert!(form_field_html.contains("error-slide"));
        assert!(!form_field_html.contains("-helper"));
        assert!(!form_field_html.contains(r#"id="-error""#));
        assert_eq!(
            formfield::FormFieldProps::default()
                .with_aria_described_by("external-help")
                .generate_described_by(),
            "external-help"
        );

        assert!(
            gamification::DifficultyBadgeProps::default()
                .with_level("easy")
                .build_classes()
                .contains("bg-success/10")
        );

        let image_html = image::Image(&image::ImageProps::default().with_loading("")).into_string();
        assert!(image_html.contains("<img"));
        assert!(!image_html.contains("loading="));

        assert!(
            indicators::TimerProps {
                size: shared::SIZE_XL,
                ..indicators::TimerProps::default()
            }
            .build_classes()
            .contains("text-2xl")
        );
        assert_eq!(
            indicators::PaginationProps {
                current_page: 5,
                total_pages: 10,
                max_visible: 5,
                ..indicators::PaginationProps::default()
            }
            .visible_pages(),
            vec![3, 4, 5, 6, 7]
        );
        assert!(
            indicators::SimpleSVGSpinner(shared::SIZE_XL, "spin")
                .into_string()
                .contains("w-10 h-10")
        );
        assert!(
            indicators::QuizResultFragment(1, 3, false)
                .into_string()
                .contains("Quiz failed")
        );

        assert!(
            input::floatingGroupClasses(&input::InputProps::default().with_error(true, "Bad"))
                .contains("is-invalid")
        );
        let floating_error_html = input::FloatingInput(
            &input::InputProps::default().with_error(true, "Bad"),
            "Label",
            "Help",
        )
        .into_string();
        assert!(floating_error_html.contains(">Bad</p>"));
        assert!(!floating_error_html.contains(">Help</p>"));
        assert!(
            input::CommaSeparatedEntryClasses(&input::InputProps {
                size: shared::SIZE_LG,
                ..input::InputProps::default()
            })
            .contains("text-base")
        );

        let label_props = label::LabelProps::default()
            .with_required(false)
            .with_optional(false);
        assert!(!label_props.required);
        assert!(!label_props.optional);

        assert!(
            megadropdown::MegaDropdownTwoColumn("left-col", "right-col",)
                .into_string()
                .contains("left-col")
        );

        let menu_summary_attrs = menu::MenuSummaryProps::default()
            .with_aria_label("Open menu")
            .build_attributes();
        assert_eq!(
            menu_summary_attrs.get("aria-label").map(String::as_str),
            Some("Open menu")
        );

        let nav_attrs = nav::NavProps::default()
            .with_aria_label("Primary")
            .with_aria_described_by("nav-help")
            .build_attributes();
        assert_eq!(
            nav_attrs.get("aria-label").map(String::as_str),
            Some("Primary")
        );
        assert_eq!(
            nav_attrs.get("aria-describedby").map(String::as_str),
            Some("nav-help")
        );
        assert!(
            !navigationarrow::NavigationArrowProps::default()
                .build_attributes()
                .contains_key("data-on:click")
        );
        assert!(
            !playbutton::PlayButtonProps::default()
                .build_attributes()
                .contains_key("data-on:click")
        );

        assert!(
            popover::PopoverProps::default()
                .with_id("pop")
                .build_attributes()
                .contains_key("id")
        );
        let popover_trigger_attrs = popover::PopoverTriggerProps {
            type_attr: String::new(),
            popover_id: "pop".to_string(),
            aria_label: "Open".to_string(),
            ..popover::PopoverTriggerProps::default()
        }
        .build_trigger_attributes();
        assert!(!popover_trigger_attrs.contains_key("type"));
        assert_eq!(
            popover_trigger_attrs
                .get("data-popover-trigger")
                .map(String::as_str),
            Some("pop")
        );
        let popover_content_attrs = popover::PopoverContentProps {
            role: String::new(),
            position: "",
            ..popover::PopoverContentProps::default()
        }
        .build_content_attributes();
        assert!(!popover_content_attrs.contains_key("role"));
        assert!(!popover_content_attrs.contains_key("data-popover-position"));
        assert!(
            popover::PopoverTitleProps::default()
                .with_id("title")
                .build_attributes()
                .contains_key("id")
        );

        let select_attr_html = selectfield::Select(
            &selectfield::SelectProps::default()
                .with_attribute("aria-describedby", "external-help"),
            &[selectfield::SelectOption::new("a", "A")],
        )
        .into_string();
        assert!(select_attr_html.contains(r#"aria-describedby="external-help""#));
        let select_error_html = selectfield::SelectWithGroups(
            &selectfield::SelectProps::default().with_error(true, "Bad"),
            &[selectfield::SelectOptionGroup::new(
                "Group",
                vec![selectfield::SelectOption::new("a", "A")],
            )],
        )
        .into_string();
        assert!(select_error_html.contains(r#"aria-invalid="true""#));
        assert!(!select_error_html.contains(r#"aria-describedby=""#));

        assert_eq!(
            skeleton::SkeletonProps {
                variant: "circle",
                width: String::new(),
                height: "2rem".to_string(),
                ..skeleton::SkeletonProps::default()
            }
            .get_style(),
            "width: 2rem; height: 2rem;"
        );
        assert!(
            skeleton::SkeletonProps {
                width: "10rem".to_string(),
                height: "4rem".to_string(),
                ..skeleton::SkeletonProps::default()
            }
            .get_style()
            .contains("height: 4rem;")
        );

        let switch_attrs = switch::SwitchProps {
            id: "toggle".to_string(),
            checked: true,
            disabled: true,
            ..switch::SwitchProps::default()
        }
        .build_attributes();
        assert_eq!(switch_attrs.get("id").map(String::as_str), Some("toggle"));
        assert_eq!(
            switch_attrs.get("aria-checked").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            switch_attrs.get("disabled").map(String::as_str),
            Some("disabled")
        );
        let switch_html = switch::Switch(&switch::SwitchProps::default()).into_string();
        assert!(!switch_html.contains("value="));

        assert_eq!(switchfield::camel_to_kebab("URLValue"), "u-r-l-value");

        let tabs_attrs = tabs::TabsProps {
            orientation: String::new(),
            ..tabs::TabsProps::default()
        }
        .build_attributes();
        assert!(!tabs_attrs.contains_key("data-orientation"));
        let tab_content_attrs = tabs::TabContentProps::default().build_attributes();
        assert!(!tab_content_attrs.contains_key("data-value"));

        let textarea_html = textarea::Textarea(&textarea::TextareaProps {
            rows: 0,
            ..textarea::TextareaProps::default()
        })
        .into_string();
        assert!(!textarea_html.contains("rows="));
        assert!(
            textarea::TextareaWithCharCount(
                &textarea::TextareaProps {
                    value: "abcd".to_string(),
                    max_length: 4,
                    show_char_count: true,
                    ..textarea::TextareaProps::default()
                },
                "Message"
            )
            .into_string()
            .contains("text-destructive")
        );

        let toast_attrs = toast::ToastProps {
            id: "toast-1".to_string(),
            aria_label: "Saved".to_string(),
            aria_described_by: "toast-desc".to_string(),
            role: String::new(),
            ..toast::ToastProps::default()
        }
        .build_attributes();
        assert_eq!(
            toast_attrs.get("data-toast-id").map(String::as_str),
            Some("toast-1")
        );
        assert_eq!(
            toast_attrs.get("aria-label").map(String::as_str),
            Some("Saved")
        );
        assert!(!toast_attrs.contains_key("role"));
        let toast_container_attrs = toast::ToastContainerProps {
            aria_live: String::new(),
            role: String::new(),
            ..toast::ToastContainerProps::default()
        }
        .build_container_attributes();
        assert_eq!(
            toast_container_attrs.get("aria-label").map(String::as_str),
            Some("Notifications")
        );
        assert!(!toast_container_attrs.contains_key("aria-live"));
        assert!(!toast_container_attrs.contains_key("role"));

        assert!(
            tooltip::TooltipProps::default()
                .with_id("tip")
                .build_attributes()
                .contains_key("id")
        );
        let tooltip_trigger_attrs = tooltip::TooltipTriggerProps::default()
            .with_tooltip_id("tip")
            .build_trigger_attributes();
        assert_eq!(
            tooltip_trigger_attrs
                .get("data-tooltip-trigger")
                .map(String::as_str),
            Some("tip")
        );
        let tooltip_content_attrs = tooltip::TooltipContentProps::default()
            .with_delay(100)
            .build_content_attributes();
        assert_eq!(
            tooltip_content_attrs
                .get("data-tooltip-delay")
                .map(String::as_str),
            Some("100")
        );

        let chat_input_count_html = chatinput::ChatInput(&chatinput::ChatInputProps {
            show_char_count: true,
            max_length: 25,
            ..chatinput::ChatInputProps::default()
        })
        .into_string();
        assert!(chat_input_count_html.contains("0 / 25"));

        let field_helper_html = field::Field(
            &field::FieldProps::default()
                .with_id("email")
                .with_helper_text("Help"),
            html! { input; },
        )
        .into_string();
        assert!(field_helper_html.contains(r#"id="email-helper""#));

        assert_eq!(
            formfield::FormFieldProps::default().generate_described_by(),
            ""
        );
        let form_field_helper_html = formfield::FormField(
            &formfield::FormFieldProps::default()
                .with_for("email")
                .with_helper_text("Help"),
            html! { input; },
        )
        .into_string();
        assert!(form_field_helper_html.contains(r#"id="email-helper""#));

        assert!(
            indicators::TimerProps {
                size: shared::SIZE_SM,
                ..indicators::TimerProps::default()
            }
            .build_classes()
            .contains("text-sm")
        );
        assert!(
            indicators::TimerProps {
                size: shared::SIZE_LG,
                ..indicators::TimerProps::default()
            }
            .build_classes()
            .contains("text-lg")
        );

        assert!(!input::floatingGroupClasses(&input::InputProps::default()).contains("is-invalid"));

        let empty_columns_html = megadropdown::MegaDropdownTwoColumn("", "").into_string();
        assert!(empty_columns_html.contains("<div></div><div></div>"));

        let menu_default_attrs = menu::MenuSummaryProps::default().build_attributes();
        assert!(!menu_default_attrs.contains_key("aria-label"));
        let nav_default_attrs = nav::NavProps::default().build_attributes();
        assert!(!nav_default_attrs.contains_key("aria-label"));

        let popover_default_attrs = popover::PopoverProps::default().build_attributes();
        assert!(!popover_default_attrs.contains_key("id"));
        let popover_default_trigger_attrs =
            popover::PopoverTriggerProps::default().build_trigger_attributes();
        assert!(!popover_default_trigger_attrs.contains_key("data-popover-trigger"));
        let popover_default_title_attrs = popover::PopoverTitleProps::default().build_attributes();
        assert!(!popover_default_title_attrs.contains_key("id"));

        let select_group_attr_html = selectfield::SelectWithGroups(
            &selectfield::SelectProps::default().with_attribute("aria-describedby", "group-help"),
            &[selectfield::SelectOptionGroup::new(
                "Group",
                vec![selectfield::SelectOption::new("a", "A")],
            )],
        )
        .into_string();
        assert!(select_group_attr_html.contains(r#"aria-describedby="group-help""#));

        assert_eq!(
            skeleton::SkeletonProps {
                variant: "circle",
                width: "3rem".to_string(),
                ..skeleton::SkeletonProps::default()
            }
            .get_style(),
            "width: 3rem; height: 3rem;"
        );
        assert_eq!(
            skeleton::SkeletonProps {
                variant: "circle",
                width: String::new(),
                height: String::new(),
                ..skeleton::SkeletonProps::default()
            }
            .get_style(),
            ""
        );
        assert_eq!(
            skeleton::SkeletonProps {
                width: String::new(),
                height: String::new(),
                ..skeleton::SkeletonProps::default()
            }
            .get_style(),
            ""
        );

        let toast_empty_attrs = toast::ToastProps {
            aria_live: String::new(),
            ..toast::ToastProps::default()
        }
        .build_attributes();
        assert!(!toast_empty_attrs.contains_key("aria-live"));
        assert!(!toast_empty_attrs.contains_key("data-toast-id"));

        let tooltip_default_attrs = tooltip::TooltipProps::default().build_attributes();
        assert!(!tooltip_default_attrs.contains_key("id"));
        let tooltip_default_trigger_attrs =
            tooltip::TooltipTriggerProps::default().build_trigger_attributes();
        assert!(!tooltip_default_trigger_attrs.contains_key("data-tooltip-trigger"));
        let tooltip_content_no_position_attrs = tooltip::TooltipContentProps {
            position: "",
            ..tooltip::TooltipContentProps::default()
        }
        .build_content_attributes();
        assert!(!tooltip_content_no_position_attrs.contains_key("data-tooltip-position"));

        assert!(
            uploadarea::UploadArea(
                &uploadarea::UploadAreaProps::new()
                    .with_title_content(html! { span { "Custom title" } },),
                html! {}
            )
            .into_string()
            .contains("Custom title")
        );
    }
}
