//! Direct-import facade for the pinned Radius configuration helpers.
const radius = @import("radius.zig");
pub const DEFAULT_RADIUS_GATEWAY = radius.DEFAULT_RADIUS_GATEWAY;
pub const RadiusGatewayModel = radius.RadiusGatewayModel;
pub const RadiusGatewayConfig = radius.RadiusGatewayConfig;
pub const RadiusOAuthCredential = radius.RadiusOAuthCredential;
pub const normalizeRadiusGatewayUrl = radius.normalizeRadiusGatewayUrl;
pub const getRadiusCredentialConfig = radius.getRadiusCredentialConfig;
pub const getRadiusModelsFromConfig = radius.getRadiusModelsFromConfig;
pub const getRadiusModels = radius.getRadiusModels;
pub const loadRadiusGatewayConfig = radius.loadRadiusGatewayConfig;
