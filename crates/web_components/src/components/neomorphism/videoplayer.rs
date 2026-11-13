#![allow(non_snake_case)]
#![allow(clippy::needless_pass_by_value)]

use crate::components::ComponentInfo;
use crate::components::unstyled::videoplayer as base;

pub use base::{VideoPlayer, VideoPlayerProps};

pub const COMPONENTS: &[ComponentInfo] = &[ComponentInfo {
    family: "neomorphism",
    module: "videoplayer",
    name: "VideoPlayer",
}];

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_neomorphism_source_components() {
        assert_eq!(COMPONENTS.len(), 1);
        assert!(COMPONENTS.iter().all(
            |component| component.family == "neomorphism" && component.module == "videoplayer"
        ));
        assert!(
            COMPONENTS
                .iter()
                .any(|component| component.name == "VideoPlayer")
        );
    }

    #[test]
    fn datastar_expression_helpers_match_templ_output() {
        let props = VideoPlayerProps::default()
            .with_state_var("player")
            .with_video_url("/video.mp4");
        assert_eq!(buildDatastarSignals(&props), "{playerPlaying: false}");
        assert_eq!(buildPlayingExpr(&props), "$playerPlaying");
        assert_eq!(buildNotPlayingExpr(&props), "!$playerPlaying");
        assert_eq!(buildPlayExpr(&props), "$playerPlaying = true");
        assert_eq!(
            buildIframeSrcExpr(&props),
            "$playerPlaying ? '/video.mp4?autoplay=1' : ''"
        );
    }
}
