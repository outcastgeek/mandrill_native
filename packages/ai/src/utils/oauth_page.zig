//! Exact pinned OAuth callback-page markup. Returned HTML belongs to a.
const std = @import("std");
fn escapeHtml(a: std.mem.Allocator, value: []const u8) ![]const u8 {
    var escaped = try a.dupe(u8, value);
    errdefer a.free(escaped);
    for ([_][2][]const u8{ .{ "&", "&amp;" }, .{ "<", "&lt;" }, .{ ">", "&gt;" }, .{ "\"", "&quot;" }, .{ "'", "&#39;" } }) |replacement| {
        const next = try std.mem.replaceOwned(u8, a, escaped, replacement[0], replacement[1]);
        a.free(escaped);
        escaped = next;
    }
    return escaped;
}
fn renderPage(a: std.mem.Allocator, title_text: []const u8, message_text: []const u8, details: ?[]const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const title = try escapeHtml(scratch, title_text);
    const heading = title;
    const message = try escapeHtml(scratch, message_text);
    const details_markup = if (details) |text| (if (text.len > 0) try std.mem.concat(scratch, u8, &.{ "<div class=\"details\">", try escapeHtml(scratch, text), "</div>" }) else "") else "";
    return std.mem.concat(a, u8, &.{
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\" />\n  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n  <title>",
        title,
        "</title>\n  <style>\n    :root {\n      --text: #fafafa;\n      --text-dim: #a1a1aa;\n      --page-bg: #09090b;\n      --font-sans: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, \"Helvetica Neue\", Arial, \"Noto Sans\", sans-serif, \"Apple Color Emoji\", \"Segoe UI Emoji\", \"Segoe UI Symbol\", \"Noto Color Emoji\";\n      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \"Liberation Mono\", \"Courier New\", monospace;\n    }\n    * { box-sizing: border-box; }\n    html { color-scheme: dark; }\n    body {\n      margin: 0;\n      min-height: 100vh;\n      display: flex;\n      align-items: center;\n      justify-content: center;\n      padding: 24px;\n      background: var(--page-bg);\n      color: var(--text);\n      font-family: var(--font-sans);\n      text-align: center;\n    }\n    main {\n      width: 100%;\n      max-width: 560px;\n      display: flex;\n      flex-direction: column;\n      align-items: center;\n      justify-content: center;\n    }\n    .logo {\n      width: 72px;\n      height: 72px;\n      display: block;\n      margin-bottom: 24px;\n    }\n    h1 {\n      margin: 0 0 10px;\n      font-size: 28px;\n      line-height: 1.15;\n      font-weight: 650;\n      color: var(--text);\n    }\n    p {\n      margin: 0;\n      line-height: 1.7;\n      color: var(--text-dim);\n      font-size: 15px;\n    }\n    .details {\n      margin-top: 16px;\n      font-family: var(--font-mono);\n      font-size: 13px;\n      color: var(--text-dim);\n      white-space: pre-wrap;\n      word-break: break-word;\n    }\n  </style>\n</head>\n<body>\n  <main>\n    <div class=\"logo\"><svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 800 800\" aria-hidden=\"true\"><path fill=\"#F09082\" d=\"M165.29 165.29H517.36V400H400V282.65H165.29Z\"/><path fill=\"#4D9ABF\" d=\"M165.29 282.65H282.65V400H400V517.36H282.65V634.72H165.29Z\"/><path fill=\"#F1BE58\" d=\"M517.36 400H634.72V634.72H517.36Z\"/></svg></div>\n    <h1>",
        heading,
        "</h1>\n    <p>",
        message,
        "</p>\n    ",
        details_markup,
        "\n  </main>\n</body>\n</html>",
    });
}
pub fn oauthSuccessHtml(a: std.mem.Allocator, message: []const u8) ![]const u8 {
    return renderPage(a, "Authentication successful", message, null);
}
pub fn oauthErrorHtml(a: std.mem.Allocator, message: []const u8, details: ?[]const u8) ![]const u8 {
    return renderPage(a, "Authentication failed", message, details);
}
