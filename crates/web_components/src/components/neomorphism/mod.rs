pub mod accordion;
pub mod activityfeed;
pub mod alert;
pub mod autocomplete;
pub mod avatar;
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
pub mod code;
pub mod collapsible;
pub mod command;
pub mod companylogo;
pub mod companylogosgrid;
pub mod componentpreview;
pub mod confirmdialog;
pub mod container;
pub mod copybutton;
pub mod dashboard;
pub mod datepicker;
pub mod datetimepicker;
pub mod democard;
pub mod dialog;
pub mod divider;
pub mod drawer;
pub mod durationbadge;
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
pub mod input;
pub mod label;
pub mod languageselector;
pub mod learning;
pub mod list;
pub mod megadropdown;
pub mod megamenufooter;
pub mod menu;
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
    components.extend_from_slice(activityfeed::COMPONENTS);
    components.extend_from_slice(alert::COMPONENTS);
    components.extend_from_slice(autocomplete::COMPONENTS);
    components.extend_from_slice(avatar::COMPONENTS);
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
    components.extend_from_slice(code::COMPONENTS);
    components.extend_from_slice(collapsible::COMPONENTS);
    components.extend_from_slice(command::COMPONENTS);
    components.extend_from_slice(companylogo::COMPONENTS);
    components.extend_from_slice(companylogosgrid::COMPONENTS);
    components.extend_from_slice(componentpreview::COMPONENTS);
    components.extend_from_slice(confirmdialog::COMPONENTS);
    components.extend_from_slice(container::COMPONENTS);
    components.extend_from_slice(copybutton::COMPONENTS);
    components.extend_from_slice(dashboard::COMPONENTS);
    components.extend_from_slice(datepicker::COMPONENTS);
    components.extend_from_slice(datetimepicker::COMPONENTS);
    components.extend_from_slice(democard::COMPONENTS);
    components.extend_from_slice(dialog::COMPONENTS);
    components.extend_from_slice(divider::COMPONENTS);
    components.extend_from_slice(drawer::COMPONENTS);
    components.extend_from_slice(durationbadge::COMPONENTS);
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
    components.extend_from_slice(input::COMPONENTS);
    components.extend_from_slice(label::COMPONENTS);
    components.extend_from_slice(languageselector::COMPONENTS);
    components.extend_from_slice(learning::COMPONENTS);
    components.extend_from_slice(list::COMPONENTS);
    components.extend_from_slice(megadropdown::COMPONENTS);
    components.extend_from_slice(megamenufooter::COMPONENTS);
    components.extend_from_slice(menu::COMPONENTS);
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
    components.extend_from_slice(switch::COMPONENTS);
    components.extend_from_slice(switchfield::COMPONENTS);
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

    #[test]
    fn all_components_includes_neomorphism_registry_entries() {
        let components = all_components();

        assert_eq!(
            components.len(),
            accordion::COMPONENTS.len()
                + activityfeed::COMPONENTS.len()
                + alert::COMPONENTS.len()
                + autocomplete::COMPONENTS.len()
                + avatar::COMPONENTS.len()
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
                + code::COMPONENTS.len()
                + collapsible::COMPONENTS.len()
                + command::COMPONENTS.len()
                + companylogo::COMPONENTS.len()
                + companylogosgrid::COMPONENTS.len()
                + componentpreview::COMPONENTS.len()
                + confirmdialog::COMPONENTS.len()
                + container::COMPONENTS.len()
                + copybutton::COMPONENTS.len()
                + dashboard::COMPONENTS.len()
                + datepicker::COMPONENTS.len()
                + datetimepicker::COMPONENTS.len()
                + democard::COMPONENTS.len()
                + dialog::COMPONENTS.len()
                + divider::COMPONENTS.len()
                + drawer::COMPONENTS.len()
                + durationbadge::COMPONENTS.len()
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
                + input::COMPONENTS.len()
                + label::COMPONENTS.len()
                + languageselector::COMPONENTS.len()
                + learning::COMPONENTS.len()
                + list::COMPONENTS.len()
                + megadropdown::COMPONENTS.len()
                + megamenufooter::COMPONENTS.len()
                + menu::COMPONENTS.len()
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
                + switch::COMPONENTS.len()
                + switchfield::COMPONENTS.len()
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
                + videoplayer::COMPONENTS.len()
        );
        assert!(
            components
                .iter()
                .all(|component| component.family == "neomorphism")
        );
        assert!(
            components
                .iter()
                .any(|component| component.module == "button")
        );
        assert!(
            components
                .iter()
                .any(|component| component.name == "MegaDropdownNav")
        );
    }
}
