//! Native OAuth entrypoint. The application owns transport, I/O, interaction,
//! and credential persistence; importing this module performs no login.
pub const types = @import("auth/types.zig");
pub const OAuthAuth = types.OAuthAuth;
pub const OAuthCredential = types.OAuthCredential;
pub const AuthInteraction = types.Interaction;
pub const LoginOptions = types.LoginOptions;
pub const anthropicOAuth = @import("auth/oauth/anthropic.zig").anthropicOAuth;
pub const openaiCodexOAuth = @import("auth/oauth/openai_codex.zig").openaiCodexOAuth;
pub const openaiChatGPTOAuth = @import("auth/oauth/openai_chatgpt.zig").openaiChatGPTOAuth;
pub const githubCopilotOAuth = @import("auth/oauth/github_copilot.zig").githubCopilotOAuth;
pub const openRouterOAuth = @import("auth/oauth/openrouter.zig").openRouterOAuth;
pub const kimiCodingOAuth = @import("auth/oauth/kimi_coding.zig").kimiCodingOAuth;
pub const metaOAuth = @import("auth/oauth/meta.zig").metaOAuth;
pub const xaiOAuth = @import("auth/oauth/xai.zig").xaiOAuth;
pub const createRadiusOAuth = @import("auth/oauth/radius.zig").createRadiusOAuth;
pub const RadiusOAuth = @import("auth/oauth/radius.zig").RadiusOAuth;
pub const pkce = @import("auth/oauth/pkce.zig");
pub const callback_server = @import("auth/oauth/callback_server.zig");
pub const device_code = @import("auth/oauth/device_code.zig");
pub const legacy = @import("auth/oauth/legacy_types.zig");
pub const OAuthAuthInfo = legacy.OAuthAuthInfo;
pub const OAuthCredentials = legacy.OAuthCredentials;
pub const OAuthDeviceCodeInfo = legacy.OAuthDeviceCodeInfo;
pub const OAuthLoginCallbacks = legacy.OAuthLoginCallbacks;
pub const OAuthPrompt = legacy.OAuthPrompt;
pub const OAuthSelectOption = legacy.OAuthSelectOption;
pub const OAuthSelectPrompt = legacy.OAuthSelectPrompt;
