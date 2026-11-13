#![allow(non_snake_case)]
#![allow(clippy::too_many_lines)]

use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ChevronLeft",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ChevronRight",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ChevronDown",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ChevronUp",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ArrowLeft",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ArrowRight",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Plus",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Edit",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Trash",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Close",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Refresh",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ThumbUp",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ThumbDown",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "MoreVertical",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Search",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Sun",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Moon",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "SearchPlus",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "SearchMinus",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Key",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Lock",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Check",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "CheckCircle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "CircleCheck",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "XCircle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "AlertTriangle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Document",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Folder",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Hive",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Image",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Slides",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Presentation",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Paperclip",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Book",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Home",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "MessageSquare",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Clipboard",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Gift",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Trophy",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Medal",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Diamond",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Sparkles",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "User",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "UserPlus",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Users",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "LogOut",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Globe",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Clock",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Calendar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Bell",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "MapPin",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "HelpCircle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "QuizPlus",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "SlidesPlus",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "StopCircle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Play",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "CloudUpload",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "CloudUploadBounce",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Stop",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "PlayFilled",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ChevronDownFilled",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "GridFourFilled",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "GraduationCap",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "BookOpen",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Monitor",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "LayoutGrid",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ArrowUp",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Robot",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "MessageCircle",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Info",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "TextAlignLeft",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "FileText",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Shield",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "ShieldCheck",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Settings",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Link2",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "CodeBrackets",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Activity",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Mail",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "AtSign",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Star",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "BarChart",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Wallet",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "CircleDollar",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "Layers",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "GridDots",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "GripDots",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "YouTubeFilled",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "LinkedInFilled",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "DiscordFilled",
    },
    ComponentInfo {
        family: "unstyled",
        module: "icons",
        name: "WhatsAppFilled",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconProps {
    pub size: &'static str,
    pub class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for IconProps {
    fn default() -> Self {
        Self {
            size: shared::SIZE_MD,
            class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl IconProps {
    #[must_use]
    pub fn with_size(mut self, size: &'static str) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn build_size_classes(&self) -> &'static str {
        size_classes(self.size)
    }

    #[must_use]
    pub fn build_classes(&self) -> String {
        shared::build_classes(&[self.build_size_classes(), self.class.as_str()])
    }

    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn size_classes(size: &str) -> &'static str {
    match size {
        shared::SIZE_XS => "w-3 h-3",
        shared::SIZE_SM => "w-4 h-4",
        shared::SIZE_LG => "w-6 h-6",
        shared::SIZE_XL => "w-8 h-8",
        _ => "w-5 h-5",
    }
}

#[must_use]
pub fn ChevronLeft(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m15 18-6-6 6-6" {}
        }
    }
}

#[must_use]
pub fn ChevronRight(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m9 18 6-6-6-6" {}
        }
    }
}

#[must_use]
pub fn ChevronDown(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
            data-class:rotate-180=[shared::attr_value(&attrs, "data-class:rotate-180")]
        {
            path d="m6 9 6 6 6-6" {}
        }
    }
}

#[must_use]
pub fn ChevronUp(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m18 15-6-6-6 6" {}
        }
    }
}

#[must_use]
pub fn ArrowLeft(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m12 19-7-7 7-7" {}
            path d="M19 12H5" {}
        }
    }
}

#[must_use]
pub fn ArrowRight(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M5 12h14" {}
            path d="m12 5 7 7-7 7" {}
        }
    }
}

#[must_use]
pub fn Plus(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M5 12h14" {}
            path d="M12 5v14" {}
        }
    }
}

#[must_use]
pub fn Edit(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" d="M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931zm0 0L19.5 7.125M18 14v4.75A2.25 2.25 0 0115.75 21H5.25A2.25 2.25 0 013 18.75V8.25A2.25 2.25 0 015.25 6H10" {}
        }
    }
}

#[must_use]
pub fn Trash(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" {}
        }
    }
}

#[must_use]
pub fn Close(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
        }
    }
}

#[must_use]
pub fn Refresh(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" {}
        }
    }
}

#[must_use]
pub fn ThumbUp(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M7 10v12" {}
            path d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2a3.13 3.13 0 0 1 3 3.88Z" {}
        }
    }
}

#[must_use]
pub fn ThumbDown(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M17 14V2" {}
            path d="M9 18.12 10 14H4.17a2 2 0 0 1-1.92-2.56l2.33-8A2 2 0 0 1 6.5 2H20a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2.76a2 2 0 0 0-1.79 1.11L12 22a3.13 3.13 0 0 1-3-3.88Z" {}
        }
    }
}

#[must_use]
pub fn MoreVertical(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="1" {}
            circle cx="12" cy="5" r="1" {}
            circle cx="12" cy="19" r="1" {}
        }
    }
}

#[must_use]
pub fn Search(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="11" cy="11" r="8" {}
            path d="m21 21-4.3-4.3" {}
        }
    }
}

#[must_use]
pub fn Sun(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="4" {}
            path d="M12 2v2" {}
            path d="M12 20v2" {}
            path d="m4.93 4.93 1.41 1.41" {}
            path d="m17.66 17.66 1.41 1.41" {}
            path d="M2 12h2" {}
            path d="M20 12h2" {}
            path d="m6.34 17.66-1.41 1.41" {}
            path d="m19.07 4.93-1.41 1.41" {}
        }
    }
}

#[must_use]
pub fn Moon(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12 3a7 7 0 1 0 9 9 9 9 0 1 1-9-9Z" {}
        }
    }
}

#[must_use]
pub fn SearchPlus(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M21 21l-6-6m2-5a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z" {}
            path d="M10 7v6" {}
            path d="M13 10H7" {}
        }
    }
}

#[must_use]
pub fn SearchMinus(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M21 21l-6-6m2-5a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z" {}
            path d="M7 10h6" {}
        }
    }
}

#[must_use]
pub fn Key(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="7.5" cy="15.5" r="5.5" {}
            path d="m21 2-9.6 9.6" {}
            path d="m15.5 7.5 3 3" {}
            path d="m18 5 3 3" {}
        }
    }
}

#[must_use]
pub fn Lock(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect x="4" y="11" width="16" height="10" rx="2" {}
            path d="M8 11V7a4 4 0 1 1 8 0v4" {}
        }
    }
}

#[must_use]
pub fn Check(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            polyline points="20 6 9 17 4 12" {}
        }
    }
}

#[must_use]
pub fn CheckCircle(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7" {}
        }
    }
}

#[must_use]
pub fn CircleCheck(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4" {}
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" {}
        }
    }
}

#[must_use]
pub fn XCircle(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="10" {}
            line x1="15" y1="9" x2="9" y2="15" {}
            line x1="9" y1="9" x2="15" y2="15" {}
        }
    }
}

#[must_use]
pub fn AlertTriangle(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" {}
            line x1="12" y1="9" x2="12" y2="13" {}
            line x1="12" y1="17" x2="12.01" y2="17" {}
        }
    }
}

#[must_use]
pub fn Document(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" {}
            polyline points="14 2 14 8 20 8" {}
        }
    }
}

#[must_use]
pub fn Folder(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M20 19a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2Z" {}
        }
    }
}

#[must_use]
pub fn Hive(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m12 2 6.5 3.75v7.5L12 17l-6.5-3.75v-7.5L12 2Z" {}
            path d="m12 2 6.5 3.75L12 9.5 5.5 5.75 12 2Z" {}
            path d="M12 9.5V17" {}
            path d="m5.5 5.75 6.5 3.75 6.5-3.75" {}
        }
    }
}

#[must_use]
pub fn Image(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect x="3" y="3" width="18" height="18" rx="2" {}
            circle cx="8.5" cy="8.5" r="1.5" {}
            path d="m21 15-5-5L5 21" {}
        }
    }
}

#[must_use]
pub fn Slides(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect width="18" height="18" x="3" y="3" rx="2" {}
            path d="M7 7h3v3H7z" {}
            path d="M7 14h3v3H7z" {}
            path d="M14 7h3" {}
            path d="M14 11h3" {}
            path d="M14 15h3" {}
        }
    }
}

#[must_use]
pub fn Presentation(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" d="M3.75 3v11.25A2.25 2.25 0 006 16.5h2.25M3.75 3h-1.5m1.5 0h16.5m0 0h1.5m-1.5 0v11.25A2.25 2.25 0 0118 16.5h-2.25m-7.5 0h7.5m-7.5 0l-1 3m8.5-3l1 3m0 0l.5 1.5m-.5-1.5h-9.5m0 0l-.5 1.5" {}
        }
    }
}

#[must_use]
pub fn Paperclip(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" {}
        }
    }
}

#[must_use]
pub fn Book(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20" {}
        }
    }
}

#[must_use]
pub fn Home(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" {}
            polyline points="9 22 9 12 15 12 15 22" {}
        }
    }
}

#[must_use]
pub fn MessageSquare(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" {}
        }
    }
}

#[must_use]
pub fn Clipboard(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" {}
            rect width="8" height="4" x="8" y="2" rx="1" ry="1" {}
        }
    }
}

#[must_use]
pub fn Gift(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            polyline points="20 12 20 22 4 22 4 12" {}
            rect width="20" height="5" x="2" y="7" {}
            line x1="12" y1="22" x2="12" y2="7" {}
            path d="M12 7H7.5a2.5 2.5 0 0 1 0-5C11 2 12 7 12 7z" {}
            path d="M12 7h4.5a2.5 2.5 0 0 0 0-5C13 2 12 7 12 7z" {}
        }
    }
}

#[must_use]
pub fn Trophy(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M8 21h8" {}
            path d="M12 17v4" {}
            path d="M7 4h10v4a5 5 0 0 1-10 0V4Z" {}
            path d="M5 6H3v1a4 4 0 0 0 4 4" {}
            path d="M19 6h2v1a4 4 0 0 1-4 4" {}
        }
    }
}

#[must_use]
pub fn Medal(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m12 14 4.5-8h3L15 14" {}
            path d="M12 14 7.5 6h-3L9 14" {}
            circle cx="12" cy="17" r="4" {}
            path d="m10.8 17 0.9 0.9 1.5-1.8" {}
        }
    }
}

#[must_use]
pub fn Diamond(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M6 4h12l4 5-10 11L2 9l4-5Z" {}
            path d="m6 4 6 16 6-16" {}
            path d="M2 9h20" {}
        }
    }
}

#[must_use]
pub fn Sparkles(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m12 3 1.7 4.3L18 9l-4.3 1.7L12 15l-1.7-4.3L6 9l4.3-1.7L12 3Z" {}
            path d="m19 14 0.9 2.1L22 17l-2.1 0.9L19 20l-0.9-2.1L16 17l2.1-0.9L19 14Z" {}
            path d="m5 15 0.7 1.6L7.3 17l-1.6 0.7L5 19.3l-0.7-1.6L2.7 17l1.6-0.4L5 15Z" {}
        }
    }
}

#[must_use]
pub fn User(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2" {}
            circle cx="12" cy="7" r="4" {}
        }
    }
}

#[must_use]
pub fn UserPlus(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" {}
            circle cx="9" cy="7" r="4" {}
            line x1="19" y1="8" x2="19" y2="14" {}
            line x1="22" y1="11" x2="16" y2="11" {}
        }
    }
}

#[must_use]
pub fn Users(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" {}
            circle cx="9" cy="7" r="4" {}
            path d="M22 21v-2a4 4 0 0 0-3-3.87" {}
            path d="M16 3.13a4 4 0 0 1 0 7.75" {}
        }
    }
}

#[must_use]
pub fn LogOut(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" {}
            polyline points="16 17 21 12 16 7" {}
            line x1="21" y1="12" x2="9" y2="12" {}
        }
    }
}

#[must_use]
pub fn Globe(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="10" {}
            line x1="2" y1="12" x2="22" y2="12" {}
            path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" {}
        }
    }
}

#[must_use]
pub fn Clock(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="10" {}
            polyline points="12 6 12 12 16 14" {}
        }
    }
}

#[must_use]
pub fn Calendar(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect width="18" height="18" x="3" y="4" rx="2" ry="2" {}
            line x1="16" y1="2" x2="16" y2="6" {}
            line x1="8" y1="2" x2="8" y2="6" {}
            line x1="3" y1="10" x2="21" y2="10" {}
        }
    }
}

#[must_use]
pub fn Bell(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M15 17h5l-1.405-1.405A2.032 2.032 0 0 1 18 14.158V11a6.002 6.002 0 0 0-4-5.659V5a2 2 0 1 0-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5" {}
            path d="M9 17a3 3 0 0 0 6 0" {}
        }
    }
}

#[must_use]
pub fn MapPin(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12 22s-7-5.6-7-12a7 7 0 1 1 14 0c0 6.4-7 12-7 12Z" {}
            circle cx="12" cy="10" r="2.5" {}
        }
    }
}

#[must_use]
pub fn HelpCircle(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="10" {}
            path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" {}
            line x1="12" y1="17" x2="12.01" y2="17" {}
        }
    }
}

#[must_use]
pub fn QuizPlus(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7" {}
            path d="M14 2v6h6" {}
            path d="M20 11V8l-6-6" {}
            path d="M9.5 10.5a2.25 2.25 0 0 1 4.5.1c0 1.5-2 1.9-2 3" {}
            path d="M12 17h.01" {}
            path d="M18 15v6" {}
            path d="M15 18h6" {}
        }
    }
}

#[must_use]
pub fn SlidesPlus(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M3 4h18" {}
            path d="M5 4v10a2 2 0 0 0 2 2h5" {}
            path d="M19 4v8" {}
            path d="M9 20l2-4" {}
            path d="M14 16l1 2" {}
            path d="M8 9h6" {}
            path d="M8 12h3" {}
            path d="M18 15v6" {}
            path d="M15 18h6" {}
        }
    }
}

#[must_use]
pub fn StopCircle(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="10" {}
            rect x="9" y="9" width="6" height="6" rx="1" {}
        }
    }
}

#[must_use]
pub fn Play(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            polygon points="5 3 19 12 5 21 5 3" {}
        }
    }
}

#[must_use]
pub fn CloudUpload(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242" {}
            path d="M12 12v9" {}
            path d="m16 16-4-4-4 4" {}
        }
    }
}

#[must_use]
pub fn CloudUploadBounce(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" {}
        }
    }
}

#[must_use]
pub fn Stop(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" d="M5.25 7.5A2.25 2.25 0 017.5 5.25h9a2.25 2.25 0 012.25 2.25v9a2.25 2.25 0 01-2.25 2.25h-9a2.25 2.25 0 01-2.25-2.25v-9z" {}
        }
    }
}

#[must_use]
pub fn PlayFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            polygon points="5 3 19 12 5 21 5 3" {}
        }
    }
}

#[must_use]
pub fn ChevronDownFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path fill-rule="evenodd" d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z" clip-rule="evenodd" {}
        }
    }
}

#[must_use]
pub fn GridFourFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect x="1.5" y="1.5" width="5.5" height="5.5" rx="1.5" fill="currentColor" {}
            rect x="9" y="1.5" width="5.5" height="5.5" rx="1.5" fill="currentColor" opacity="0.58" {}
            rect x="1.5" y="9" width="5.5" height="5.5" rx="1.5" fill="currentColor" opacity="0.58" {}
            rect x="9" y="9" width="5.5" height="5.5" rx="1.5" fill="currentColor" opacity="0.28" {}
        }
    }
}

#[must_use]
pub fn GraduationCap(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" d="M4.26 10.147a60.438 60.438 0 0 0-.491 6.347A48.627 48.627 0 0 1 12 20.904a48.627 48.627 0 0 1 8.232-4.41 60.46 60.46 0 0 0-.491-6.347m-15.482 0a50.57 50.57 0 0 0-2.658-.813A59.906 59.906 0 0 1 12 3.493a59.903 59.903 0 0 1 10.399 5.84c-.896.248-1.783.52-2.658.814m-15.482 0A50.697 50.697 0 0 1 12 13.489a50.702 50.702 0 0 1 7.74-3.342M6.75 15a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm0 0v-3.675A55.378 55.378 0 0 1 12 8.443m-7.007 11.55A5.981 5.981 0 0 0 6.75 15.75v-1.5" {}
        }
    }
}

#[must_use]
pub fn BookOpen(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" d="M12 6.042A8.967 8.967 0 006 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 016 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 016-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0018 18a8.967 8.967 0 00-6 2.292m0-14.25v14.25" {}
        }
    }
}

#[must_use]
pub fn Monitor(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect width="18" height="14" x="3" y="5" rx="2" {}
            path d="M7 15h10" {}
            path d="M7 11h4" {}
        }
    }
}

#[must_use]
pub fn LayoutGrid(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect x="3" y="3" width="7" height="7" {}
            rect x="14" y="3" width="7" height="7" {}
            rect x="14" y="14" width="7" height="7" {}
            rect x="3" y="14" width="7" height="7" {}
        }
    }
}

#[must_use]
pub fn ArrowUp(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="m5 12 7-7 7 7" {}
            path d="M12 19V5" {}
        }
    }
}

#[must_use]
pub fn Robot(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12 8V4H8" {}
            rect width="16" height="12" x="4" y="8" rx="2" {}
            path d="M2 14h2" {}
            path d="M20 14h2" {}
            path d="M15 13v2" {}
            path d="M9 13v2" {}
        }
    }
}

#[must_use]
pub fn MessageCircle(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z" {}
        }
    }
}

#[must_use]
pub fn Info(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="10" {}
            path d="M12 16v-4" {}
            path d="M12 8h.01" {}
        }
    }
}

#[must_use]
pub fn TextAlignLeft(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M4 6h16" {}
            path d="M4 12h16" {}
            path d="M4 18h7" {}
        }
    }
}

#[must_use]
pub fn FileText(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" {}
        }
    }
}

#[must_use]
pub fn Shield(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" {}
        }
    }
}

#[must_use]
pub fn ShieldCheck(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" {}
            path d="m9 12 2 2 4-4" {}
        }
    }
}

#[must_use]
pub fn Settings(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" {}
            circle cx="12" cy="12" r="3" {}
        }
    }
}

#[must_use]
pub fn Link2(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M9 17H7A5 5 0 0 1 7 7h2" {}
            path d="M15 7h2a5 5 0 1 1 0 10h-2" {}
            line x1="8" x2="16" y1="12" y2="12" {}
        }
    }
}

#[must_use]
pub fn CodeBrackets(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            polyline points="16 18 22 12 16 6" {}
            polyline points="8 6 2 12 8 18" {}
        }
    }
}

#[must_use]
pub fn Activity(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M22 12h-4l-3 9L9 3l-3 9H2" {}
        }
    }
}

#[must_use]
pub fn Mail(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect width="20" height="16" x="2" y="4" rx="2" {}
            path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7" {}
        }
    }
}

#[must_use]
pub fn AtSign(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="12" cy="12" r="4" {}
            path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8" {}
        }
    }
}

#[must_use]
pub fn Star(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" {}
        }
    }
}

#[must_use]
pub fn BarChart(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            line x1="12" x2="12" y1="20" y2="10" {}
            line x1="18" x2="18" y1="20" y2="4" {}
            line x1="6" x2="6" y1="20" y2="16" {}
        }
    }
}

#[must_use]
pub fn Wallet(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M2.25 18.75a60.07 60.07 0 0115.797 2.101c.727.198 1.453-.342 1.453-1.096V18.75M3.75 4.5v.75A.75.75 0 013 6h-.75m0 0v-.375c0-.621.504-1.125 1.125-1.125H20.25M2.25 6v9m18-10.5v.75c0 .414.336.75.75.75h.75m-1.5-1.5h.375c.621 0 1.125.504 1.125 1.125v9.75c0 .621-.504 1.125-1.125 1.125h-.375m1.5-1.5H21a.75.75 0 00-.75.75v.75m0 0H3.75m0 0h-.375a1.125 1.125 0 01-1.125-1.125V15m1.5 1.5v-.75A.75.75 0 003 15h-.75M15 10.5a3 3 0 11-6 0 3 3 0 016 0zm3 0h.008v.008H18V10.5zm-12 0h.008v.008H6V10.5z" {}
        }
    }
}

#[must_use]
pub fn CircleDollar(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" {}
        }
    }
}

#[must_use]
pub fn Layers(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M12 2 2 7l10 5 10-5-10-5Z" {}
            path d="m2 17 10 5 10-5" {}
            path d="m2 12 10 5 10-5" {}
        }
    }
}

#[must_use]
pub fn GridDots(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) fill="none" stroke="currentColor" viewBox="0 0 24 24" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            rect x="2" y="2" width="20" height="20" rx="2" {}
            path d="M8 10h.01M12 10h.01M16 10h.01M8 14h.01M12 14h.01M16 14h.01" {}
        }
    }
}

#[must_use]
pub fn GripDots(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 16" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            circle cx="2.5" cy="2.5" r="1.5" {}
            circle cx="7.5" cy="2.5" r="1.5" {}
            circle cx="2.5" cy="8" r="1.5" {}
            circle cx="7.5" cy="8" r="1.5" {}
            circle cx="2.5" cy="13.5" r="1.5" {}
            circle cx="7.5" cy="13.5" r="1.5" {}
        }
    }
}

#[must_use]
pub fn YouTubeFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814ZM9.545 15.568V8.432L15.818 12l-6.273 3.568Z" {}
        }
    }
}

#[must_use]
pub fn LinkedInFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286ZM5.337 7.433a2.062 2.062 0 0 1-2.063-2.065 2.064 2.064 0 1 1 2.063 2.065Zm1.782 13.019H3.555V9h3.564v11.452ZM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003Z" {}
        }
    }
}

#[must_use]
pub fn DiscordFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03Z" {}
        }
    }
}

#[must_use]
pub fn WhatsAppFilled(props: &IconProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        svg class=(props.build_classes()) xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"
            id=[shared::attr_value(&attrs, "id")]
            title=[shared::attr_value(&attrs, "title")]
            role=[shared::attr_value(&attrs, "role")]
            aria-label=[shared::attr_value(&attrs, "aria-label")]
            aria-hidden=[shared::attr_value(&attrs, "aria-hidden")]
            aria-describedby=[shared::attr_value(&attrs, "aria-describedby")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
            data-on:click=[shared::attr_value(&attrs, "data-on:click")]
            data-on:mouseenter=[shared::attr_value(&attrs, "data-on:mouseenter")]
            data-on:mouseleave=[shared::attr_value(&attrs, "data-on:mouseleave")]
            data-on:focus=[shared::attr_value(&attrs, "data-on:focus")]
            data-on:blur=[shared::attr_value(&attrs, "data-on:blur")]
            data-show=[shared::attr_value(&attrs, "data-show")]
            data-text=[shared::attr_value(&attrs, "data-text")]
            data-signals=[shared::attr_value(&attrs, "data-signals")]
            data-init=[shared::attr_value(&attrs, "data-init")]
            data-class:active=[shared::attr_value(&attrs, "data-class:active")]
        {
            path d="M20.52 3.48A11.86 11.86 0 0 0 12.07 0C5.49 0 .14 5.35.14 11.93c0 2.1.55 4.14 1.6 5.94L0 24l6.32-1.66a11.9 11.9 0 0 0 5.75 1.47h.01c6.58 0 11.93-5.35 11.93-11.93 0-3.19-1.24-6.19-3.49-8.4Zm-8.45 18.32h-.01a9.9 9.9 0 0 1-5.04-1.37l-.36-.21-3.75.98 1-3.65-.23-.37a9.9 9.9 0 0 1-1.52-5.25c0-5.47 4.45-9.92 9.93-9.92 2.65 0 5.14 1.03 7.01 2.91a9.86 9.86 0 0 1 2.91 7c0 5.48-4.45 9.93-9.92 9.93Zm5.44-7.44c-.3-.15-1.78-.88-2.06-.98-.27-.1-.47-.15-.66.15-.2.3-.76.98-.94 1.18-.17.2-.35.22-.65.08-.3-.15-1.26-.46-2.39-1.46-.88-.78-1.48-1.75-1.66-2.04-.17-.3-.02-.46.13-.6.14-.14.3-.35.45-.53.15-.17.2-.3.3-.5.1-.2.05-.38-.02-.53-.08-.15-.66-1.6-.9-2.18-.24-.58-.48-.5-.66-.5h-.56c-.2 0-.52.08-.8.38-.27.3-1.04 1.02-1.04 2.48 0 1.45 1.07 2.86 1.22 3.05.15.2 2.1 3.21 5.09 4.5.71.3 1.27.48 1.7.61.72.23 1.37.2 1.88.12.58-.09 1.78-.73 2.03-1.44.25-.71.25-1.31.18-1.44-.07-.12-.27-.2-.57-.35Z" {}
        }
    }
}

#[must_use]
pub fn normalize_symbol(symbol: &str) -> String {
    symbol
        .trim()
        .replace('\u{FE0F}', "")
        .replace('\u{FE0E}', "")
}

#[must_use]
pub fn IconFromAlias(alias: &str, props: &IconProps) -> Option<Markup> {
    let normalized = alias.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "bar_chart" | "chart" | "analytics" | "stats" | "bar" | "graph" => Some(BarChart(props)),
        "chat" | "message" | "messages" | "bubble" => Some(MessageCircle(props)),
        "note" | "memo" | "annotation" | "document" | "file" => Some(FileText(props)),
        "book" | "books" => Some(BookOpen(props)),
        "link" | "links" => Some(Link2(props)),
        "clipboard" | "tasks" => Some(Clipboard(props)),
        "quiz" | "question" | "quiz_plus" | "question_plus" => Some(QuizPlus(props)),
        "check" | "check_circle" | "ok" => Some(CheckCircle(props)),
        "users" | "user" | "people" | "profile" => Some(Users(props)),
        "presentation" | "slides" | "slide" => Some(Presentation(props)),
        "slides_plus" | "presentation_plus" => Some(SlidesPlus(props)),
        "dashboard" | "grid" | "layout" => Some(LayoutGrid(props)),
        "graduation" | "graduation_cap" | "student" => Some(GraduationCap(props)),
        "settings" | "gear" => Some(Settings(props)),
        "activity" | "flare" | "lightning" | "bolt" | "spark" => Some(Activity(props)),
        "wallet" | "money" | "billing" | "currency" => Some(Wallet(props)),
        "mail" | "mailbox" | "envelope" => Some(Mail(props)),
        "lock" | "locked" | "unlock" => Some(Lock(props)),
        "search" | "find" => Some(Search(props)),
        "sun" | "light" | "brightness" => Some(Sun(props)),
        "moon" | "dark" | "night" => Some(Moon(props)),
        "monitor" | "system" | "desktop" => Some(Monitor(props)),
        "calendar" | "date" => Some(Calendar(props)),
        "bell" | "notification" | "notifications" => Some(Bell(props)),
        "upload" | "cloud_upload" => Some(CloudUpload(props)),
        "clock" => Some(Clock(props)),
        "thumb_up" | "thumbs_up" | "like" => Some(ThumbUp(props)),
        "thumb_down" | "thumbs_down" | "dislike" => Some(ThumbDown(props)),
        "code" | "snippet" | "shortcode" => Some(CodeBrackets(props)),
        "star" | "target" => Some(Star(props)),
        "trophy" => Some(Trophy(props)),
        "medal" => Some(Medal(props)),
        "diamond" | "gem" => Some(Diamond(props)),
        "sparkle" | "sparkles" => Some(Sparkles(props)),
        "home" | "house" => Some(Home(props)),
        "warning" | "alert" => Some(AlertTriangle(props)),
        "close" | "error" | "wrong" | "bad" => Some(XCircle(props)),
        "gift" | "bonus" | "reward" => Some(Gift(props)),
        "log_out" | "logout" | "exit" | "signout" => Some(LogOut(props)),
        "stop_circle" | "end_session" => Some(StopCircle(props)),
        "fire" => Some(Activity(props)),
        _ => None,
    }
}

#[must_use]
pub fn IconFromSymbol(symbol: &str, props: &IconProps) -> Option<Markup> {
    let value = normalize_symbol(symbol);
    if value.is_empty() {
        return None;
    }
    if let Some(icon) = IconFromAlias(value.as_str(), props) {
        return Some(icon);
    }
    match value.as_str() {
        "🎓" => Some(GraduationCap(props)),
        "🔗" | "🔖" => Some(Link2(props)),
        "📊" | "📈" | "📉" => Some(BarChart(props)),
        "🔔" => Some(Bell(props)),
        "📚" | "📖" => Some(BookOpen(props)),
        "👥" | "👤" | "👨‍🎓" | "🧑" | "👨" | "👩" => Some(Users(props)),
        "🔐" | "🔑" | "🔒" | "🔓" => Some(Lock(props)),
        "📢" | "📣" => Some(MessageCircle(props)),
        "💰" | "🪙" | "💳" => Some(Wallet(props)),
        "🛠" | "⚙" | "🔧" => Some(Settings(props)),
        "📋" | "🗒" | "🧾" => Some(Clipboard(props)),
        "🔎" | "🔍" => Some(Search(props)),
        "💬" | "💭" | "🗨" => Some(MessageCircle(props)),
        "🏠" => Some(Home(props)),
        "🚪" => Some(LogOut(props)),
        "📌" | "📍" => Some(MapPin(props)),
        "🧩" | "🧠" | "🟢" | "⚡" => Some(Activity(props)),
        "✨" => Some(Sparkles(props)),
        "🔥" => Some(Activity(props)),
        "⚠" | "💥" => Some(AlertTriangle(props)),
        "✅" => Some(CheckCircle(props)),
        "❌" | "⛔" => Some(XCircle(props)),
        "🎁" | "🎉" => Some(Gift(props)),
        "💎" => Some(Diamond(props)),
        "🎭" => Some(Users(props)),
        "🎯" | "⭐" => Some(Star(props)),
        "🌟" => Some(Sparkles(props)),
        "📝" | "📄" => Some(FileText(props)),
        "🧱" | "🪟" => Some(LayoutGrid(props)),
        "🏆" => Some(Trophy(props)),
        "🏅" | "🎖" => Some(Medal(props)),
        "📅" => Some(Calendar(props)),
        "📤" => Some(CloudUpload(props)),
        "🎞" | "🎬" => Some(Presentation(props)),
        "📬" | "📭" | "✉" | "📧" => Some(Mail(props)),
        "🚀" => Some(Play(props)),
        "⏱" | "🕒" | "🕑" | "🕰" => Some(Clock(props)),
        "🗣" => Some(MessageSquare(props)),
        "🧪" => Some(Presentation(props)),
        "👁" | "👀" => Some(MessageCircle(props)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type IconRenderer = fn(&IconProps) -> Markup;

    const ICON_CASES: &[(&str, IconRenderer, &str)] = &[
        (
            "ChevronLeft",
            ChevronLeft,
            "<path d=\"m15 18-6-6 6-6\"></path>",
        ),
        (
            "ChevronRight",
            ChevronRight,
            "<path d=\"m9 18 6-6-6-6\"></path>",
        ),
        (
            "ChevronDown",
            ChevronDown,
            "<path d=\"m6 9 6 6 6-6\"></path>",
        ),
        ("ChevronUp", ChevronUp, "<path d=\"m18 15-6-6-6 6\"></path>"),
        ("ArrowLeft", ArrowLeft, "<path d=\"m12 19-7-7 7-7\"></path>"),
        ("ArrowRight", ArrowRight, "<path d=\"M5 12h14\"></path>"),
        ("Plus", Plus, "<path d=\"M5 12h14\"></path>"),
        (
            "Edit",
            Edit,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931zm0 0L19.5 7.125M18 14v4.75A2.25 2.25 0 0115.75 21H5.25A2.25 2.25 0 013 18.75V8.25A2.25 2.25 0 015.25 6H10\"></path>",
        ),
        (
            "Trash",
            Trash,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16\"></path>",
        ),
        (
            "Close",
            Close,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M6 18L18 6M6 6l12 12\"></path>",
        ),
        (
            "Refresh",
            Refresh,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15\"></path>",
        ),
        ("ThumbUp", ThumbUp, "<path d=\"M7 10v12\"></path>"),
        ("ThumbDown", ThumbDown, "<path d=\"M17 14V2\"></path>"),
        (
            "MoreVertical",
            MoreVertical,
            "<circle cx=\"12\" cy=\"12\" r=\"1\"></circle>",
        ),
        (
            "Search",
            Search,
            "<circle cx=\"11\" cy=\"11\" r=\"8\"></circle>",
        ),
        ("Sun", Sun, "<circle cx=\"12\" cy=\"12\" r=\"4\"></circle>"),
        (
            "Moon",
            Moon,
            "<path d=\"M12 3a7 7 0 1 0 9 9 9 9 0 1 1-9-9Z\"></path>",
        ),
        (
            "SearchPlus",
            SearchPlus,
            "<path d=\"M21 21l-6-6m2-5a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z\"></path>",
        ),
        (
            "SearchMinus",
            SearchMinus,
            "<path d=\"M21 21l-6-6m2-5a7 7 0 1 1-14 0 7 7 0 0 1 14 0Z\"></path>",
        ),
        (
            "Key",
            Key,
            "<circle cx=\"7.5\" cy=\"15.5\" r=\"5.5\"></circle>",
        ),
        (
            "Lock",
            Lock,
            "<rect x=\"4\" y=\"11\" width=\"16\" height=\"10\" rx=\"2\"></rect>",
        ),
        (
            "Check",
            Check,
            "<polyline points=\"20 6 9 17 4 12\"></polyline>",
        ),
        (
            "CheckCircle",
            CheckCircle,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2.5\" d=\"M5 13l4 4L19 7\"></path>",
        ),
        (
            "CircleCheck",
            CircleCheck,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M9 12l2 2 4-4\"></path>",
        ),
        (
            "XCircle",
            XCircle,
            "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
        ),
        (
            "AlertTriangle",
            AlertTriangle,
            "<path d=\"m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z\"></path>",
        ),
        (
            "Document",
            Document,
            "<path d=\"M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z\"></path>",
        ),
        (
            "Folder",
            Folder,
            "<path d=\"M20 19a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2Z\"></path>",
        ),
        (
            "Hive",
            Hive,
            "<path d=\"m12 2 6.5 3.75v7.5L12 17l-6.5-3.75v-7.5L12 2Z\"></path>",
        ),
        (
            "Image",
            Image,
            "<rect x=\"3\" y=\"3\" width=\"18\" height=\"18\" rx=\"2\"></rect>",
        ),
        (
            "Slides",
            Slides,
            "<rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\"></rect>",
        ),
        (
            "Presentation",
            Presentation,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M3.75 3v11.25A2.25 2.25 0 006 16.5h2.25M3.75 3h-1.5m1.5 0h16.5m0 0h1.5m-1.5 0v11.25A2.25 2.25 0 0118 16.5h-2.25m-7.5 0h7.5m-7.5 0l-1 3m8.5-3l1 3m0 0l.5 1.5m-.5-1.5h-9.5m0 0l-.5 1.5\"></path>",
        ),
        (
            "Paperclip",
            Paperclip,
            "<path d=\"m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48\"></path>",
        ),
        (
            "Book",
            Book,
            "<path d=\"M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20\"></path>",
        ),
        (
            "Home",
            Home,
            "<path d=\"M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z\"></path>",
        ),
        (
            "MessageSquare",
            MessageSquare,
            "<path d=\"M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z\"></path>",
        ),
        (
            "Clipboard",
            Clipboard,
            "<path d=\"M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2\"></path>",
        ),
        (
            "Gift",
            Gift,
            "<polyline points=\"20 12 20 22 4 22 4 12\"></polyline>",
        ),
        ("Trophy", Trophy, "<path d=\"M8 21h8\"></path>"),
        ("Medal", Medal, "<path d=\"m12 14 4.5-8h3L15 14\"></path>"),
        (
            "Diamond",
            Diamond,
            "<path d=\"M6 4h12l4 5-10 11L2 9l4-5Z\"></path>",
        ),
        (
            "Sparkles",
            Sparkles,
            "<path d=\"m12 3 1.7 4.3L18 9l-4.3 1.7L12 15l-1.7-4.3L6 9l4.3-1.7L12 3Z\"></path>",
        ),
        (
            "User",
            User,
            "<path d=\"M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2\"></path>",
        ),
        (
            "UserPlus",
            UserPlus,
            "<path d=\"M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2\"></path>",
        ),
        (
            "Users",
            Users,
            "<path d=\"M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2\"></path>",
        ),
        (
            "LogOut",
            LogOut,
            "<path d=\"M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4\"></path>",
        ),
        (
            "Globe",
            Globe,
            "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
        ),
        (
            "Clock",
            Clock,
            "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
        ),
        (
            "Calendar",
            Calendar,
            "<rect width=\"18\" height=\"18\" x=\"3\" y=\"4\" rx=\"2\" ry=\"2\"></rect>",
        ),
        (
            "Bell",
            Bell,
            "<path d=\"M15 17h5l-1.405-1.405A2.032 2.032 0 0 1 18 14.158V11a6.002 6.002 0 0 0-4-5.659V5a2 2 0 1 0-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5\"></path>",
        ),
        (
            "MapPin",
            MapPin,
            "<path d=\"M12 22s-7-5.6-7-12a7 7 0 1 1 14 0c0 6.4-7 12-7 12Z\"></path>",
        ),
        (
            "HelpCircle",
            HelpCircle,
            "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
        ),
        (
            "QuizPlus",
            QuizPlus,
            "<path d=\"M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7\"></path>",
        ),
        ("SlidesPlus", SlidesPlus, "<path d=\"M3 4h18\"></path>"),
        (
            "StopCircle",
            StopCircle,
            "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
        ),
        (
            "Play",
            Play,
            "<polygon points=\"5 3 19 12 5 21 5 3\"></polygon>",
        ),
        (
            "CloudUpload",
            CloudUpload,
            "<path d=\"M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242\"></path>",
        ),
        (
            "CloudUploadBounce",
            CloudUploadBounce,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12\"></path>",
        ),
        (
            "Stop",
            Stop,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M5.25 7.5A2.25 2.25 0 017.5 5.25h9a2.25 2.25 0 012.25 2.25v9a2.25 2.25 0 01-2.25 2.25h-9a2.25 2.25 0 01-2.25-2.25v-9z\"></path>",
        ),
        (
            "PlayFilled",
            PlayFilled,
            "<polygon points=\"5 3 19 12 5 21 5 3\"></polygon>",
        ),
        (
            "ChevronDownFilled",
            ChevronDownFilled,
            "<path fill-rule=\"evenodd\" d=\"M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z\" clip-rule=\"evenodd\"></path>",
        ),
        (
            "GridFourFilled",
            GridFourFilled,
            "<rect x=\"1.5\" y=\"1.5\" width=\"5.5\" height=\"5.5\" rx=\"1.5\" fill=\"currentColor\"></rect>",
        ),
        (
            "GraduationCap",
            GraduationCap,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M4.26 10.147a60.438 60.438 0 0 0-.491 6.347A48.627 48.627 0 0 1 12 20.904a48.627 48.627 0 0 1 8.232-4.41 60.46 60.46 0 0 0-.491-6.347m-15.482 0a50.57 50.57 0 0 0-2.658-.813A59.906 59.906 0 0 1 12 3.493a59.903 59.903 0 0 1 10.399 5.84c-.896.248-1.783.52-2.658.814m-15.482 0A50.697 50.697 0 0 1 12 13.489a50.702 50.702 0 0 1 7.74-3.342M6.75 15a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm0 0v-3.675A55.378 55.378 0 0 1 12 8.443m-7.007 11.55A5.981 5.981 0 0 0 6.75 15.75v-1.5\"></path>",
        ),
        (
            "BookOpen",
            BookOpen,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M12 6.042A8.967 8.967 0 006 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 016 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 016-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0018 18a8.967 8.967 0 00-6 2.292m0-14.25v14.25\"></path>",
        ),
        (
            "Monitor",
            Monitor,
            "<rect width=\"18\" height=\"14\" x=\"3\" y=\"5\" rx=\"2\"></rect>",
        ),
        (
            "LayoutGrid",
            LayoutGrid,
            "<rect x=\"3\" y=\"3\" width=\"7\" height=\"7\"></rect>",
        ),
        ("ArrowUp", ArrowUp, "<path d=\"m5 12 7-7 7 7\"></path>"),
        ("Robot", Robot, "<path d=\"M12 8V4H8\"></path>"),
        (
            "MessageCircle",
            MessageCircle,
            "<path d=\"M7.9 20A9 9 0 1 0 4 16.1L2 22Z\"></path>",
        ),
        (
            "Info",
            Info,
            "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
        ),
        (
            "TextAlignLeft",
            TextAlignLeft,
            "<path d=\"M4 6h16\"></path>",
        ),
        (
            "FileText",
            FileText,
            "<path d=\"M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z\"></path>",
        ),
        (
            "Shield",
            Shield,
            "<path d=\"M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z\"></path>",
        ),
        (
            "ShieldCheck",
            ShieldCheck,
            "<path d=\"M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z\"></path>",
        ),
        (
            "Settings",
            Settings,
            "<path d=\"M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z\"></path>",
        ),
        (
            "Link2",
            Link2,
            "<path d=\"M9 17H7A5 5 0 0 1 7 7h2\"></path>",
        ),
        (
            "CodeBrackets",
            CodeBrackets,
            "<polyline points=\"16 18 22 12 16 6\"></polyline>",
        ),
        (
            "Activity",
            Activity,
            "<path d=\"M22 12h-4l-3 9L9 3l-3 9H2\"></path>",
        ),
        (
            "Mail",
            Mail,
            "<rect width=\"20\" height=\"16\" x=\"2\" y=\"4\" rx=\"2\"></rect>",
        ),
        (
            "AtSign",
            AtSign,
            "<circle cx=\"12\" cy=\"12\" r=\"4\"></circle>",
        ),
        (
            "Star",
            Star,
            "<polygon points=\"12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2\"></polygon>",
        ),
        (
            "BarChart",
            BarChart,
            "<line x1=\"12\" x2=\"12\" y1=\"20\" y2=\"10\"></line>",
        ),
        (
            "Wallet",
            Wallet,
            "<path d=\"M2.25 18.75a60.07 60.07 0 0115.797 2.101c.727.198 1.453-.342 1.453-1.096V18.75M3.75 4.5v.75A.75.75 0 013 6h-.75m0 0v-.375c0-.621.504-1.125 1.125-1.125H20.25M2.25 6v9m18-10.5v.75c0 .414.336.75.75.75h.75m-1.5-1.5h.375c.621 0 1.125.504 1.125 1.125v9.75c0 .621-.504 1.125-1.125 1.125h-.375m1.5-1.5H21a.75.75 0 00-.75.75v.75m0 0H3.75m0 0h-.375a1.125 1.125 0 01-1.125-1.125V15m1.5 1.5v-.75A.75.75 0 003 15h-.75M15 10.5a3 3 0 11-6 0 3 3 0 016 0zm3 0h.008v.008H18V10.5zm-12 0h.008v.008H6V10.5z\"></path>",
        ),
        (
            "CircleDollar",
            CircleDollar,
            "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2\" d=\"M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z\"></path>",
        ),
        (
            "Layers",
            Layers,
            "<path d=\"M12 2 2 7l10 5 10-5-10-5Z\"></path>",
        ),
        (
            "GridDots",
            GridDots,
            "<rect x=\"2\" y=\"2\" width=\"20\" height=\"20\" rx=\"2\"></rect>",
        ),
        (
            "GripDots",
            GripDots,
            "<circle cx=\"2.5\" cy=\"2.5\" r=\"1.5\"></circle>",
        ),
        (
            "YouTubeFilled",
            YouTubeFilled,
            "<path d=\"M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814ZM9.545 15.568V8.432L15.818 12l-6.273 3.568Z\"></path>",
        ),
        (
            "LinkedInFilled",
            LinkedInFilled,
            "<path d=\"M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286ZM5.337 7.433a2.062 2.062 0 0 1-2.063-2.065 2.064 2.064 0 1 1 2.063 2.065Zm1.782 13.019H3.555V9h3.564v11.452ZM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003Z\"></path>",
        ),
        (
            "DiscordFilled",
            DiscordFilled,
            "<path d=\"M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03Z\"></path>",
        ),
        (
            "WhatsAppFilled",
            WhatsAppFilled,
            "<path d=\"M20.52 3.48A11.86 11.86 0 0 0 12.07 0C5.49 0 .14 5.35.14 11.93c0 2.1.55 4.14 1.6 5.94L0 24l6.32-1.66a11.9 11.9 0 0 0 5.75 1.47h.01c6.58 0 11.93-5.35 11.93-11.93 0-3.19-1.24-6.19-3.49-8.4Zm-8.45 18.32h-.01a9.9 9.9 0 0 1-5.04-1.37l-.36-.21-3.75.98 1-3.65-.23-.37a9.9 9.9 0 0 1-1.52-5.25c0-5.47 4.45-9.92 9.93-9.92 2.65 0 5.14 1.03 7.01 2.91a9.86 9.86 0 0 1 2.91 7c0 5.48-4.45 9.93-9.92 9.93Zm5.44-7.44c-.3-.15-1.78-.88-2.06-.98-.27-.1-.47-.15-.66.15-.2.3-.76.98-.94 1.18-.17.2-.35.22-.65.08-.3-.15-1.26-.46-2.39-1.46-.88-.78-1.48-1.75-1.66-2.04-.17-.3-.02-.46.13-.6.14-.14.3-.35.45-.53.15-.17.2-.3.3-.5.1-.2.05-.38-.02-.53-.08-.15-.66-1.6-.9-2.18-.24-.58-.48-.5-.66-.5h-.56c-.2 0-.52.08-.8.38-.27.3-1.04 1.02-1.04 2.48 0 1.45 1.07 2.86 1.22 3.05.15.2 2.1 3.21 5.09 4.5.71.3 1.27.48 1.7.61.72.23 1.37.2 1.88.12.58-.09 1.78-.73 2.03-1.44.25-.71.25-1.31.18-1.44-.07-.12-.27-.2-.57-.35Z\"></path>",
        ),
    ];

    #[test]
    fn component_registry_lists_every_icon_function() {
        assert_eq!(COMPONENTS.len(), ICON_CASES.len());
        for (component, (name, _, _)) in COMPONENTS.iter().zip(ICON_CASES.iter()) {
            assert_eq!(component.family, "unstyled");
            assert_eq!(component.module, "icons");
            assert_eq!(component.name, *name);
        }
    }

    #[test]
    fn icon_props_helpers_cover_all_sizes_classes_and_attributes() {
        let cases = [
            (shared::SIZE_XS, "w-3 h-3"),
            (shared::SIZE_SM, "w-4 h-4"),
            (shared::SIZE_MD, "w-5 h-5"),
            (shared::SIZE_LG, "w-6 h-6"),
            (shared::SIZE_XL, "w-8 h-8"),
            ("unknown", "w-5 h-5"),
        ];
        for (size, expected) in cases {
            let props = IconProps::default().with_size(size);
            assert_eq!(size_classes(size), expected);
            assert_eq!(props.build_size_classes(), expected);
        }
        let props = IconProps::default()
            .with_class("text-primary shrink-0")
            .with_attribute("data-testid", "icon-1");
        assert_eq!(props.build_classes(), "w-5 h-5 text-primary shrink-0");
        assert_eq!(
            props
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("icon-1")
        );
    }

    #[test]
    fn every_icon_renders_real_svg_children_from_the_templ_source() {
        for (name, render, expected_child) in ICON_CASES {
            let html = render(&IconProps::default()).into_string();
            assert!(
                html.starts_with("<svg class=\"w-5 h-5\""),
                "{name} did not render the default svg class: {html}"
            );
            assert!(
                html.contains("viewBox=\""),
                "{name} did not render an SVG viewBox: {html}"
            );
            assert!(
                html.contains(expected_child),
                "{name} did not render its source child {expected_child}: {html}"
            );
            assert!(
                !html.contains("<svg></svg>"),
                "{name} rendered an empty stub svg: {html}"
            );
        }
    }

    #[test]
    fn icon_attributes_render_with_maud_escaping_and_datastar_colons() {
        let props = IconProps::default()
            .with_size(shared::SIZE_LG)
            .with_class("text-info")
            .with_attribute("id", "search-icon")
            .with_attribute("title", "Find <items>")
            .with_attribute("aria-label", "Search \"items\"")
            .with_attribute("data-on:click", "$open = true")
            .with_attribute("data-show", "$visible");
        let html = Search(&props).into_string();
        assert!(html.contains("class=\"w-6 h-6 text-info\""));
        assert!(html.contains("id=\"search-icon\""));
        assert!(html.contains("title=\"Find &lt;items&gt;\""));
        assert!(html.contains("aria-label=\"Search &quot;items&quot;\""));
        assert!(html.contains("data-on:click=\"$open = true\""));
        assert!(html.contains("data-show=\"$visible\""));
        assert!(!html.contains("data-on-click"));
    }

    #[test]
    fn aliases_cover_every_declared_alias_and_render_expected_glyphs() {
        let cases: &[(&[&str], &str)] = &[
            (
                &["bar_chart", "chart", "analytics", "stats", "bar", "graph"],
                "<line x1=\"12\" x2=\"12\" y1=\"20\" y2=\"10\"></line>",
            ),
            (
                &["chat", "message", "messages", "bubble"],
                "<path d=\"M7.9 20A9 9 0 1 0 4 16.1L2 22Z\"></path>",
            ),
            (
                &["note", "memo", "annotation", "document", "file"],
                "<path d=\"M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z\"></path>",
            ),
            (
                &["book", "books"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M12 6.042A8.967 8.967 0 006 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 016 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 016-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0018 18a8.967 8.967 0 00-6 2.292m0-14.25v14.25\"></path>",
            ),
            (
                &["link", "links"],
                "<path d=\"M9 17H7A5 5 0 0 1 7 7h2\"></path>",
            ),
            (
                &["clipboard", "tasks"],
                "<path d=\"M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2\"></path>",
            ),
            (
                &["quiz", "question", "quiz_plus", "question_plus"],
                "<path d=\"M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7\"></path>",
            ),
            (
                &["check", "check_circle", "ok"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2.5\" d=\"M5 13l4 4L19 7\"></path>",
            ),
            (
                &["users", "user", "people", "profile"],
                "<path d=\"M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2\"></path>",
            ),
            (
                &["presentation", "slides", "slide"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M3.75 3v11.25A2.25 2.25 0 006 16.5h2.25M3.75 3h-1.5m1.5 0h16.5m0 0h1.5m-1.5 0v11.25A2.25 2.25 0 0118 16.5h-2.25m-7.5 0h7.5m-7.5 0l-1 3m8.5-3l1 3m0 0l.5 1.5m-.5-1.5h-9.5m0 0l-.5 1.5\"></path>",
            ),
            (
                &["slides_plus", "presentation_plus"],
                "<path d=\"M3 4h18\"></path>",
            ),
            (
                &["dashboard", "grid", "layout"],
                "<rect x=\"3\" y=\"3\" width=\"7\" height=\"7\"></rect>",
            ),
            (
                &["graduation", "graduation_cap", "student"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M4.26 10.147a60.438 60.438 0 0 0-.491 6.347A48.627 48.627 0 0 1 12 20.904a48.627 48.627 0 0 1 8.232-4.41 60.46 60.46 0 0 0-.491-6.347m-15.482 0a50.57 50.57 0 0 0-2.658-.813A59.906 59.906 0 0 1 12 3.493a59.903 59.903 0 0 1 10.399 5.84c-.896.248-1.783.52-2.658.814m-15.482 0A50.697 50.697 0 0 1 12 13.489a50.702 50.702 0 0 1 7.74-3.342M6.75 15a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm0 0v-3.675A55.378 55.378 0 0 1 12 8.443m-7.007 11.55A5.981 5.981 0 0 0 6.75 15.75v-1.5\"></path>",
            ),
            (
                &["settings", "gear"],
                "<path d=\"M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z\"></path>",
            ),
            (
                &["activity", "flare", "lightning", "bolt", "spark"],
                "<path d=\"M22 12h-4l-3 9L9 3l-3 9H2\"></path>",
            ),
            (
                &["wallet", "money", "billing", "currency"],
                "<path d=\"M2.25 18.75a60.07 60.07 0 0115.797 2.101c.727.198 1.453-.342 1.453-1.096V18.75M3.75 4.5v.75A.75.75 0 013 6h-.75m0 0v-.375c0-.621.504-1.125 1.125-1.125H20.25M2.25 6v9m18-10.5v.75c0 .414.336.75.75.75h.75m-1.5-1.5h.375c.621 0 1.125.504 1.125 1.125v9.75c0 .621-.504 1.125-1.125 1.125h-.375m1.5-1.5H21a.75.75 0 00-.75.75v.75m0 0H3.75m0 0h-.375a1.125 1.125 0 01-1.125-1.125V15m1.5 1.5v-.75A.75.75 0 003 15h-.75M15 10.5a3 3 0 11-6 0 3 3 0 016 0zm3 0h.008v.008H18V10.5zm-12 0h.008v.008H6V10.5z\"></path>",
            ),
            (
                &["mail", "mailbox", "envelope"],
                "<rect width=\"20\" height=\"16\" x=\"2\" y=\"4\" rx=\"2\"></rect>",
            ),
            (
                &["lock", "locked", "unlock"],
                "<rect x=\"4\" y=\"11\" width=\"16\" height=\"10\" rx=\"2\"></rect>",
            ),
            (
                &["search", "find"],
                "<circle cx=\"11\" cy=\"11\" r=\"8\"></circle>",
            ),
            (
                &["sun", "light", "brightness"],
                "<circle cx=\"12\" cy=\"12\" r=\"4\"></circle>",
            ),
            (
                &["moon", "dark", "night"],
                "<path d=\"M12 3a7 7 0 1 0 9 9 9 9 0 1 1-9-9Z\"></path>",
            ),
            (
                &["monitor", "system", "desktop"],
                "<rect width=\"18\" height=\"14\" x=\"3\" y=\"5\" rx=\"2\"></rect>",
            ),
            (
                &["calendar", "date"],
                "<rect width=\"18\" height=\"18\" x=\"3\" y=\"4\" rx=\"2\" ry=\"2\"></rect>",
            ),
            (
                &["bell", "notification", "notifications"],
                "<path d=\"M15 17h5l-1.405-1.405A2.032 2.032 0 0 1 18 14.158V11a6.002 6.002 0 0 0-4-5.659V5a2 2 0 1 0-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5\"></path>",
            ),
            (
                &["upload", "cloud_upload"],
                "<path d=\"M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242\"></path>",
            ),
            (&["clock"], "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>"),
            (
                &["thumb_up", "thumbs_up", "like"],
                "<path d=\"M7 10v12\"></path>",
            ),
            (
                &["thumb_down", "thumbs_down", "dislike"],
                "<path d=\"M17 14V2\"></path>",
            ),
            (
                &["code", "snippet", "shortcode"],
                "<polyline points=\"16 18 22 12 16 6\"></polyline>",
            ),
            (
                &["star", "target"],
                "<polygon points=\"12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2\"></polygon>",
            ),
            (&["trophy"], "<path d=\"M8 21h8\"></path>"),
            (&["medal"], "<path d=\"m12 14 4.5-8h3L15 14\"></path>"),
            (
                &["diamond", "gem"],
                "<path d=\"M6 4h12l4 5-10 11L2 9l4-5Z\"></path>",
            ),
            (
                &["sparkle", "sparkles"],
                "<path d=\"m12 3 1.7 4.3L18 9l-4.3 1.7L12 15l-1.7-4.3L6 9l4.3-1.7L12 3Z\"></path>",
            ),
            (
                &["home", "house"],
                "<path d=\"M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z\"></path>",
            ),
            (
                &["warning", "alert"],
                "<path d=\"m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z\"></path>",
            ),
            (
                &["close", "error", "wrong", "bad"],
                "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
            ),
            (
                &["gift", "bonus", "reward"],
                "<polyline points=\"20 12 20 22 4 22 4 12\"></polyline>",
            ),
            (
                &["log_out", "logout", "exit", "signout"],
                "<path d=\"M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4\"></path>",
            ),
            (
                &["stop_circle", "end_session"],
                "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
            ),
            (&["fire"], "<path d=\"M22 12h-4l-3 9L9 3l-3 9H2\"></path>"),
        ];
        for (aliases, expected_child) in cases {
            for alias in *aliases {
                let icon = IconFromAlias(alias, &IconProps::default());
                assert!(icon.is_some(), "alias {alias} did not resolve");
                let html = icon.unwrap().into_string();
                assert!(
                    html.contains(expected_child),
                    "alias {alias} rendered the wrong glyph: {html}"
                );
            }
        }
        assert!(IconFromAlias("missing", &IconProps::default()).is_none());
        assert!(IconFromAlias(" thumb-up ", &IconProps::default()).is_some());
    }

    #[test]
    fn symbols_cover_every_declared_symbol_and_render_expected_glyphs() {
        let cases: &[(&[&str], &str)] = &[
            (
                &["🎓"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M4.26 10.147a60.438 60.438 0 0 0-.491 6.347A48.627 48.627 0 0 1 12 20.904a48.627 48.627 0 0 1 8.232-4.41 60.46 60.46 0 0 0-.491-6.347m-15.482 0a50.57 50.57 0 0 0-2.658-.813A59.906 59.906 0 0 1 12 3.493a59.903 59.903 0 0 1 10.399 5.84c-.896.248-1.783.52-2.658.814m-15.482 0A50.697 50.697 0 0 1 12 13.489a50.702 50.702 0 0 1 7.74-3.342M6.75 15a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm0 0v-3.675A55.378 55.378 0 0 1 12 8.443m-7.007 11.55A5.981 5.981 0 0 0 6.75 15.75v-1.5\"></path>",
            ),
            (&["🔗", "🔖"], "<path d=\"M9 17H7A5 5 0 0 1 7 7h2\"></path>"),
            (
                &["📊", "📈", "📉"],
                "<line x1=\"12\" x2=\"12\" y1=\"20\" y2=\"10\"></line>",
            ),
            (
                &["🔔"],
                "<path d=\"M15 17h5l-1.405-1.405A2.032 2.032 0 0 1 18 14.158V11a6.002 6.002 0 0 0-4-5.659V5a2 2 0 1 0-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5\"></path>",
            ),
            (
                &["📚", "📖"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M12 6.042A8.967 8.967 0 006 3.75c-1.052 0-2.062.18-3 .512v14.25A8.987 8.987 0 016 18c2.305 0 4.408.867 6 2.292m0-14.25a8.966 8.966 0 016-2.292c1.052 0 2.062.18 3 .512v14.25A8.987 8.987 0 0018 18a8.967 8.967 0 00-6 2.292m0-14.25v14.25\"></path>",
            ),
            (
                &["👥", "👤", "👨‍🎓", "🧑", "👨", "👩"],
                "<path d=\"M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2\"></path>",
            ),
            (
                &["🔐", "🔑", "🔒", "🔓"],
                "<rect x=\"4\" y=\"11\" width=\"16\" height=\"10\" rx=\"2\"></rect>",
            ),
            (
                &["📢", "📣"],
                "<path d=\"M7.9 20A9 9 0 1 0 4 16.1L2 22Z\"></path>",
            ),
            (
                &["💰", "🪙", "💳"],
                "<path d=\"M2.25 18.75a60.07 60.07 0 0115.797 2.101c.727.198 1.453-.342 1.453-1.096V18.75M3.75 4.5v.75A.75.75 0 013 6h-.75m0 0v-.375c0-.621.504-1.125 1.125-1.125H20.25M2.25 6v9m18-10.5v.75c0 .414.336.75.75.75h.75m-1.5-1.5h.375c.621 0 1.125.504 1.125 1.125v9.75c0 .621-.504 1.125-1.125 1.125h-.375m1.5-1.5H21a.75.75 0 00-.75.75v.75m0 0H3.75m0 0h-.375a1.125 1.125 0 01-1.125-1.125V15m1.5 1.5v-.75A.75.75 0 003 15h-.75M15 10.5a3 3 0 11-6 0 3 3 0 016 0zm3 0h.008v.008H18V10.5zm-12 0h.008v.008H6V10.5z\"></path>",
            ),
            (
                &["🛠", "🛠️", "⚙", "⚙️", "🔧"],
                "<path d=\"M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z\"></path>",
            ),
            (
                &["📋", "🗒", "🧾"],
                "<path d=\"M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2\"></path>",
            ),
            (
                &["🔎", "🔍"],
                "<circle cx=\"11\" cy=\"11\" r=\"8\"></circle>",
            ),
            (
                &["💬", "💭", "🗨", "🗨️"],
                "<path d=\"M7.9 20A9 9 0 1 0 4 16.1L2 22Z\"></path>",
            ),
            (
                &["🏠"],
                "<path d=\"M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z\"></path>",
            ),
            (
                &["🚪"],
                "<path d=\"M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4\"></path>",
            ),
            (
                &["📌", "📍"],
                "<path d=\"M12 22s-7-5.6-7-12a7 7 0 1 1 14 0c0 6.4-7 12-7 12Z\"></path>",
            ),
            (
                &["🧩", "🧠", "🟢", "⚡"],
                "<path d=\"M22 12h-4l-3 9L9 3l-3 9H2\"></path>",
            ),
            (
                &["✨"],
                "<path d=\"m12 3 1.7 4.3L18 9l-4.3 1.7L12 15l-1.7-4.3L6 9l4.3-1.7L12 3Z\"></path>",
            ),
            (&["🔥"], "<path d=\"M22 12h-4l-3 9L9 3l-3 9H2\"></path>"),
            (
                &["⚠", "⚠️", "💥"],
                "<path d=\"m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z\"></path>",
            ),
            (
                &["✅"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-width=\"2.5\" d=\"M5 13l4 4L19 7\"></path>",
            ),
            (
                &["❌", "⛔"],
                "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
            ),
            (
                &["🎁", "🎉"],
                "<polyline points=\"20 12 20 22 4 22 4 12\"></polyline>",
            ),
            (&["💎"], "<path d=\"M6 4h12l4 5-10 11L2 9l4-5Z\"></path>"),
            (
                &["🎭"],
                "<path d=\"M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2\"></path>",
            ),
            (
                &["🎯", "⭐"],
                "<polygon points=\"12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2\"></polygon>",
            ),
            (
                &["🌟"],
                "<path d=\"m12 3 1.7 4.3L18 9l-4.3 1.7L12 15l-1.7-4.3L6 9l4.3-1.7L12 3Z\"></path>",
            ),
            (
                &["📝", "📄"],
                "<path d=\"M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z\"></path>",
            ),
            (
                &["🧱", "🪟"],
                "<rect x=\"3\" y=\"3\" width=\"7\" height=\"7\"></rect>",
            ),
            (&["🏆"], "<path d=\"M8 21h8\"></path>"),
            (&["🏅", "🎖"], "<path d=\"m12 14 4.5-8h3L15 14\"></path>"),
            (
                &["📅"],
                "<rect width=\"18\" height=\"18\" x=\"3\" y=\"4\" rx=\"2\" ry=\"2\"></rect>",
            ),
            (
                &["📤"],
                "<path d=\"M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242\"></path>",
            ),
            (
                &["🎞", "🎞️", "🎬"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M3.75 3v11.25A2.25 2.25 0 006 16.5h2.25M3.75 3h-1.5m1.5 0h16.5m0 0h1.5m-1.5 0v11.25A2.25 2.25 0 0118 16.5h-2.25m-7.5 0h7.5m-7.5 0l-1 3m8.5-3l1 3m0 0l.5 1.5m-.5-1.5h-9.5m0 0l-.5 1.5\"></path>",
            ),
            (
                &["📬", "📭", "✉", "✉️", "📧"],
                "<rect width=\"20\" height=\"16\" x=\"2\" y=\"4\" rx=\"2\"></rect>",
            ),
            (&["🚀"], "<polygon points=\"5 3 19 12 5 21 5 3\"></polygon>"),
            (
                &["⏱", "⏱️", "🕒", "🕑", "🕰"],
                "<circle cx=\"12\" cy=\"12\" r=\"10\"></circle>",
            ),
            (
                &["🗣", "🗣️"],
                "<path d=\"M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z\"></path>",
            ),
            (
                &["🧪"],
                "<path stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M3.75 3v11.25A2.25 2.25 0 006 16.5h2.25M3.75 3h-1.5m1.5 0h16.5m0 0h1.5m-1.5 0v11.25A2.25 2.25 0 0118 16.5h-2.25m-7.5 0h7.5m-7.5 0l-1 3m8.5-3l1 3m0 0l.5 1.5m-.5-1.5h-9.5m0 0l-.5 1.5\"></path>",
            ),
            (
                &["👁", "👁️", "👀"],
                "<path d=\"M7.9 20A9 9 0 1 0 4 16.1L2 22Z\"></path>",
            ),
        ];
        for (symbols, expected_child) in cases {
            for symbol in *symbols {
                let icon = IconFromSymbol(symbol, &IconProps::default());
                assert!(icon.is_some(), "symbol {symbol} did not resolve");
                let html = icon.unwrap().into_string();
                assert!(
                    html.contains(expected_child),
                    "symbol {symbol} rendered the wrong glyph: {html}"
                );
            }
        }
        assert_eq!(normalize_symbol("  ⚙️  "), "⚙");
        assert_eq!(normalize_symbol("  \u{FE0E}  "), "");
        assert!(IconFromSymbol("", &IconProps::default()).is_none());
        assert!(IconFromSymbol("search", &IconProps::default()).is_some());
        assert!(IconFromSymbol("not-an-icon", &IconProps::default()).is_none());
    }
}
