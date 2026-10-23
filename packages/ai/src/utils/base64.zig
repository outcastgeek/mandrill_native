//! Forgiving-input boundaries composed from regex and Zig 0.17's standard
//! Base64 decoder. No alphabet lookup, bit extraction or byte codec is added.
const std = @import("std");
const regex = @import("../regex/regexp.zig");
fn decodedValidated(a: std.mem.Allocator, input: []const u8) ![]u8 {
    var symbols = input.len - std.mem.countScalar(u8, input, '=');
    for ("\t\n\x0c\r ") |ignored| symbols -= std.mem.countScalar(u8, input, ignored);
    const size = std.base64.Base64DecoderWithIgnore.init(std.base64.standard_alphabet_chars, null, "");
    const length = size.calcSizeUpperBound(symbols);
    const out = try a.alloc(u8, length);
    errdefer a.free(out);
    const decoder = std.base64.standard.decoderWithIgnore("\t\n\x0c\r ");
    // The caller has already proved the complete alphabet/padding grammar.
    // In pinned Zig 0.17, decode writes every complete output octet before
    // its final InvalidPadding check. Each caller's lexical/length boundary
    // permits only the partial tail or omitted padding its source accepts.
    // Keep the standard decoder's fully written bytes, without rewriting bits.
    _ = decoder.decode(out, input) catch |err| switch (err) {
        error.InvalidPadding => length,
        else => return err,
    };
    return out;
}
pub fn decodeAtob(a: std.mem.Allocator, input: []const u8) ![]u8 {
    if (!try regex.matches(a, null,
        \\^(?:(?:[\t\n\f\r ]*[A-Za-z0-9+/]){4})*(?:(?:[\t\n\f\r ]*[A-Za-z0-9+/]){2}(?:[\t\n\f\r ]*=){2}|(?:[\t\n\f\r ]*[A-Za-z0-9+/]){3}[\t\n\f\r ]*=|(?:[\t\n\f\r ]*[A-Za-z0-9+/]){2,3})?[\t\n\f\r ]*(?![\s\S])
    , input, .{})) return error.InvalidCharacter;
    return decodedValidated(a, input);
}
/// Smithy's fromBase64 checks raw length and its pinned lexical regex before
/// delegating to Buffer. Preserve that boundary, including regex end rules.
pub fn decodeSmithy(a: std.mem.Allocator, input: []const u8) ![]u8 {
    const unicode = @import("unicode.zig");
    if ((try unicode.utf16Length(input)) % 4 != 0) return error.InvalidPadding;
    if (!try regex.matches(a, null, "^[A-Za-z0-9+/]*={0,2}$", input, .{})) return error.InvalidCharacter;
    return decodedValidated(a, input);
}
pub fn decodeNodeBuffer(a: std.mem.Allocator, io: ?std.Io, input: []const u8) ![]u8 {
    const prefix = input[0 .. std.mem.findScalar(u8, input, '=') orelse input.len];
    var runs = try regex.Regex.init(a, io, "[A-Za-z0-9+/_-]+", .{});
    defer runs.deinit();
    var symbols = try std.Io.Writer.Allocating.initCapacity(a, prefix.len);
    defer symbols.deinit();
    defer std.crypto.secureZero(u8, @constCast(symbols.written()));
    var offset: usize = 0;
    while (try runs.execSensitive(a, prefix, offset)) |found| {
        var matched = found;
        defer matched.deinit();
        offset = matched.captures[0].?.end;
        const text = (try matched.text(a, 0)).?;
        defer {
            std.crypto.secureZero(u8, @constCast(text));
            a.free(text);
        }
        try symbols.writer.writeAll(text);
    }
    const plus = try std.mem.replaceOwned(u8, a, symbols.written(), "-", "+");
    defer {
        std.crypto.secureZero(u8, plus);
        a.free(plus);
    }
    const normalized = try std.mem.replaceOwned(u8, a, plus, "_", "/");
    defer {
        std.crypto.secureZero(u8, normalized);
        a.free(normalized);
    }
    const decoder = std.base64.Base64DecoderWithIgnore.init(std.base64.standard_alphabet_chars, null, "");
    const length = decoder.calcSizeUpperBound(normalized.len);
    const bytes = try a.alloc(u8, length);
    errdefer {
        std.crypto.secureZero(u8, bytes);
        a.free(bytes);
    }
    _ = decoder.decode(bytes, normalized) catch |err| switch (err) {
        error.InvalidPadding => length,
        else => return err,
    };
    return bytes;
}
