#![allow(non_snake_case)]

use crate::components::unstyled::playbutton::{PlayButton, PlayButtonProps};
use crate::components::unstyled::videocontainer::{VideoContainer, VideoContainerProps};
use crate::components::{ComponentInfo, shared};
use maud::{Markup, html};
use std::collections::BTreeMap;

pub const COMPONENTS: &[ComponentInfo] = &[
    ComponentInfo {
        family: "unstyled",
        module: "videoplayer",
        name: "VideoPlayer",
    },
    ComponentInfo {
        family: "unstyled",
        module: "videoplayer",
        name: "SimpleVideoPlayer",
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoPlayerProps {
    pub video_url: String,
    pub video_type: String,
    pub thumbnail_url: String,
    pub aspect_ratio: String,
    pub show_play_button: bool,
    pub play_button_size: &'static str,
    pub play_button_color: String,
    pub state_var: String,
    pub container_style: String,
    pub class: String,
    pub container_class: String,
    pub attributes: BTreeMap<String, String>,
}

impl Default for VideoPlayerProps {
    fn default() -> Self {
        Self {
            video_url: String::new(),
            video_type: "youtube".to_string(),
            thumbnail_url: String::new(),
            aspect_ratio: "16/9".to_string(),
            show_play_button: true,
            play_button_size: shared::SIZE_LG,
            play_button_color: "blue-600".to_string(),
            state_var: "videoState".to_string(),
            container_style: "rounded".to_string(),
            class: String::new(),
            container_class: String::new(),
            attributes: BTreeMap::new(),
        }
    }
}

impl VideoPlayerProps {
    #[must_use]
    pub fn with_youtube(mut self, video_id: impl Into<String>) -> Self {
        self.video_type = "youtube".to_string();
        self.video_url = format!("https://www.youtube.com/embed/{}", video_id.into());
        self
    }
    #[must_use]
    pub fn with_vimeo(mut self, video_id: impl Into<String>) -> Self {
        self.video_type = "vimeo".to_string();
        self.video_url = format!("https://player.vimeo.com/video/{}", video_id.into());
        self
    }
    #[must_use]
    pub fn with_direct(mut self, url: impl Into<String>) -> Self {
        self.video_type = "direct".to_string();
        self.video_url = url.into();
        self
    }
    #[must_use]
    pub fn with_video_url(mut self, value: impl Into<String>) -> Self {
        self.video_url = value.into();
        self
    }
    #[must_use]
    pub fn with_thumbnail(mut self, value: impl Into<String>) -> Self {
        self.thumbnail_url = value.into();
        self
    }
    #[must_use]
    pub fn with_aspect_ratio(mut self, value: impl Into<String>) -> Self {
        self.aspect_ratio = value.into();
        self
    }
    #[must_use]
    pub fn with_container_style(mut self, value: impl Into<String>) -> Self {
        self.container_style = value.into();
        self
    }
    #[must_use]
    pub fn with_state_var(mut self, value: impl Into<String>) -> Self {
        self.state_var = value.into();
        self
    }
    #[must_use]
    pub const fn with_show_play_button(mut self, value: bool) -> Self {
        self.show_play_button = value;
        self
    }
    #[must_use]
    pub const fn with_play_button_size(mut self, value: &'static str) -> Self {
        self.play_button_size = value;
        self
    }
    #[must_use]
    pub fn with_play_button_color(mut self, value: impl Into<String>) -> Self {
        self.play_button_color = value.into();
        self
    }
    #[must_use]
    pub fn with_class(mut self, value: impl Into<String>) -> Self {
        self.class = value.into();
        self
    }
    #[must_use]
    pub fn with_container_class(mut self, value: impl Into<String>) -> Self {
        self.container_class = value.into();
        self
    }
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
    #[must_use]
    pub fn build_attributes(&self) -> BTreeMap<String, String> {
        self.attributes.clone()
    }
}

#[must_use]
pub fn buildDatastarSignals(props: &VideoPlayerProps) -> String {
    format!("{{{}Playing: false}}", props.state_var)
}
#[must_use]
pub fn buildPlayingExpr(props: &VideoPlayerProps) -> String {
    format!("${}Playing", props.state_var)
}
#[must_use]
pub fn buildNotPlayingExpr(props: &VideoPlayerProps) -> String {
    format!("!${}Playing", props.state_var)
}
#[must_use]
pub fn buildPlayExpr(props: &VideoPlayerProps) -> String {
    format!("${}Playing = true", props.state_var)
}
#[must_use]
pub fn buildIframeSrcExpr(props: &VideoPlayerProps) -> String {
    format!(
        "${}Playing ? '{}?autoplay=1' : ''",
        props.state_var, props.video_url
    )
}

#[must_use]
pub fn VideoPlayer(props: &VideoPlayerProps) -> Markup {
    let attrs = props.build_attributes();
    html! {
        div
            data-signals=(buildDatastarSignals(props))
            class=(&props.class)
            id=[shared::attr_value(&attrs, "id")]
            data-testid=[shared::attr_value(&attrs, "data-testid")]
        {
            (VideoContainer(&VideoContainerProps::default()
                .with_aspect_ratio(&props.aspect_ratio)
                .with_container_style(&props.container_style)
                .with_container_class(&props.container_class),
                html! {
                    @if props.show_play_button {
                        div
                            class="absolute inset-0 flex items-center justify-center cursor-pointer z-10"
                            data-show=(buildNotPlayingExpr(props))
                            data-on:click=(buildPlayExpr(props))
                        {
                            (PlayButton(&PlayButtonProps::default()
                                .with_size(props.play_button_size)
                                .with_color(&props.play_button_color)
                                .with_on_click(buildPlayExpr(props))))
                        }
                    }
                    div
                        class="absolute inset-0 transition-opacity duration-300"
                        data-show=(buildPlayingExpr(props))
                        style="display: none"
                    {
                        iframe
                            class="w-full h-full rounded-xl"
                            frameborder="0"
                            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
                            allowfullscreen
                            data-attr:src=(buildIframeSrcExpr(props))
                        {}
                    }
                }))
        }
    }
}

#[must_use]
pub fn SimpleVideoPlayer(youtube_id: &str) -> Markup {
    VideoPlayer(&VideoPlayerProps::default().with_youtube(youtube_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_cover_defaults_builders_and_expressions() {
        let props = VideoPlayerProps::default();
        assert_eq!(props.video_type, "youtube");
        assert_eq!(props.aspect_ratio, "16/9");
        assert!(props.show_play_button);
        assert_eq!(props.play_button_size, shared::SIZE_LG);
        assert_eq!(props.play_button_color, "blue-600");
        assert_eq!(props.state_var, "videoState");
        assert_eq!(props.container_style, "rounded");
        assert_eq!(buildDatastarSignals(&props), "{videoStatePlaying: false}");
        assert_eq!(buildPlayingExpr(&props), "$videoStatePlaying");
        assert_eq!(buildNotPlayingExpr(&props), "!$videoStatePlaying");
        assert_eq!(buildPlayExpr(&props), "$videoStatePlaying = true");
        assert_eq!(
            buildIframeSrcExpr(&props),
            "$videoStatePlaying ? '?autoplay=1' : ''"
        );

        let built = VideoPlayerProps::default()
            .with_youtube("abc")
            .with_vimeo("vimeo")
            .with_direct("/video.mp4")
            .with_video_url("/custom")
            .with_thumbnail("/thumb.jpg")
            .with_aspect_ratio("1/1")
            .with_container_style("elevated")
            .with_state_var("player")
            .with_show_play_button(false)
            .with_play_button_size(shared::SIZE_SM)
            .with_play_button_color("primary")
            .with_class("outer")
            .with_container_class("inner")
            .with_attribute("data-testid", "player");
        assert_eq!(built.video_type, "direct");
        assert_eq!(built.video_url, "/custom");
        assert_eq!(built.thumbnail_url, "/thumb.jpg");
        assert_eq!(built.aspect_ratio, "1/1");
        assert_eq!(built.container_style, "elevated");
        assert_eq!(built.state_var, "player");
        assert!(!built.show_play_button);
        assert_eq!(built.play_button_size, shared::SIZE_SM);
        assert_eq!(built.play_button_color, "primary");
        assert_eq!(built.class, "outer");
        assert_eq!(built.container_class, "inner");
        assert_eq!(
            built
                .build_attributes()
                .get("data-testid")
                .map(String::as_str),
            Some("player")
        );
        assert_eq!(
            buildIframeSrcExpr(&built),
            "$playerPlaying ? '/custom?autoplay=1' : ''"
        );
        assert_eq!(
            VideoPlayerProps::default().with_youtube("yt").video_url,
            "https://www.youtube.com/embed/yt"
        );
        assert_eq!(
            VideoPlayerProps::default().with_vimeo("42").video_url,
            "https://player.vimeo.com/video/42"
        );
    }

    #[test]
    fn video_player_renders_overlay_iframe_container_and_datastar() {
        let rendered = VideoPlayer(
            &VideoPlayerProps::default()
                .with_youtube("abc")
                .with_state_var("player")
                .with_class("wrap")
                .with_container_class("inner")
                .with_attribute("data-testid", "video"),
        )
        .into_string();
        assert!(rendered.starts_with(
            "<div data-signals=\"{playerPlaying: false}\" class=\"wrap\" data-testid=\"video\">"
        ));
        assert!(
            rendered.contains(
                "video-container relative overflow-hidden rounded-2xl aspect-video inner"
            )
        );
        assert!(rendered.contains("class=\"absolute inset-0 flex items-center justify-center cursor-pointer z-10\" data-show=\"!$playerPlaying\" data-on:click=\"$playerPlaying = true\""));
        assert!(rendered.contains("aria-label=\"Play video\""));
        assert!(rendered.contains("data-on:click=\"$playerPlaying = true\""));
        assert!(rendered.contains("class=\"absolute inset-0 transition-opacity duration-300\" data-show=\"$playerPlaying\" style=\"display: none\""));
        assert!(rendered.contains("allowfullscreen data-attr:src=\"$playerPlaying ? 'https://www.youtube.com/embed/abc?autoplay=1' : ''\""));
    }

    #[test]
    fn video_player_can_hide_play_button_and_simple_player_uses_youtube() {
        let no_overlay = VideoPlayer(
            &VideoPlayerProps::default()
                .with_direct("/movie.mp4")
                .with_show_play_button(false),
        )
        .into_string();
        assert!(!no_overlay.contains("cursor-pointer z-10"));
        assert!(
            no_overlay
                .contains("data-attr:src=\"$videoStatePlaying ? '/movie.mp4?autoplay=1' : ''\"")
        );

        let simple = SimpleVideoPlayer("xyz").into_string();
        assert!(simple.contains("https://www.youtube.com/embed/xyz?autoplay=1"));
        assert!(simple.contains("data-signals=\"{videoStatePlaying: false}\""));
    }
}
