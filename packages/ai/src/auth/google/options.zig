//! The supported external-account option aliases are fixed schema fields.
//! Copy decoded tree nodes; do not transform key text or parse JSON here.
const std = @import("std");
const j = @import("../../json/value.zig");
const Value = std.json.Value;
const Alias = struct { original: []const u8, camel: []const u8 };
fn apply(a: std.mem.Allocator, value: *Value, aliases: []const Alias) !void {
    if (value.* != .object) return;
    for (aliases) |alias| {
        if (j.get(value.*, alias.original)) |existing| if (existing != .null) continue;
        if (j.get(value.*, alias.camel)) |replacement| try j.put(a, value, alias.original, replacement);
    }
}
pub fn external(a: std.mem.Allocator, original: Value) !Value {
    var result = try j.clone(a, original);
    try apply(a, &result, &.{
        .{ .original = "client_id", .camel = "clientId" },                                        .{ .original = "client_secret", .camel = "clientSecret" },
        .{ .original = "token_url", .camel = "tokenUrl" },                                        .{ .original = "subject_token_type", .camel = "subjectTokenType" },
        .{ .original = "workforce_pool_user_project", .camel = "workforcePoolUserProject" },      .{ .original = "service_account_impersonation_url", .camel = "serviceAccountImpersonationUrl" },
        .{ .original = "service_account_impersonation", .camel = "serviceAccountImpersonation" }, .{ .original = "credential_source", .camel = "credentialSource" },
        .{ .original = "quota_project_id", .camel = "quotaProjectId" },                           .{ .original = "universe_domain", .camel = "universeDomain" },
        .{ .original = "subject_token_supplier", .camel = "subjectTokenSupplier" },               .{ .original = "aws_security_credentials_supplier", .camel = "awsSecurityCredentialsSupplier" },
    });
    if (result == .object) {
        if (result.object.getPtr("service_account_impersonation")) |config| try apply(a, config, &.{.{ .original = "token_lifetime_seconds", .camel = "tokenLifetimeSeconds" }});
        if (result.object.getPtr("credential_source")) |source| {
            try apply(a, source, &.{
                .{ .original = "environment_id", .camel = "environmentId" },                   .{ .original = "region_url", .camel = "regionUrl" },
                .{ .original = "imdsv2_session_token_url", .camel = "imdsv2SessionTokenUrl" }, .{ .original = "regional_cred_verification_url", .camel = "regionalCredVerificationUrl" },
            });
            if (source.* == .object) if (source.object.getPtr("format")) |format| try apply(a, format, &.{.{ .original = "subject_token_field_name", .camel = "subjectTokenFieldName" }});
        }
    }
    return result;
}
