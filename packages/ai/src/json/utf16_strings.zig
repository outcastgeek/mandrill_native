//! Approved lone-surrogate JSON emission. Ordinary escaping stays in std.json.
const std = @import("std");
pub fn encode(text: []const u8, options: std.json.Stringify.Options, out: *std.Io.Writer) std.json.Stringify.Error!void {
    try out.writeByte('"');
    try chars(text, options, out);
    try out.writeByte('"');
}
pub fn chars(text: []const u8, options: std.json.Stringify.Options, out: *std.Io.Writer) std.json.Stringify.Error!void {
    var index: usize = 0;
    var start: usize = 0;
    while (index < text.len) {
        const n = std.unicode.utf8ByteSequenceLength(text[index]) catch return error.WriteFailed;
        if (n > text.len - index) return error.WriteFailed;
        const code = std.unicode.wtf8Decode(text[index..][0..n]) catch return error.WriteFailed;
        if (code >= 0xd800 and code <= 0xdfff) {
            try std.json.Stringify.encodeJsonStringChars(text[start..index], options, out);
            var paired = false;
            if (code <= 0xdbff and text.len - index - n >= 3) {
                const low = std.unicode.wtf8Decode(text[index + n ..][0..3]) catch 0;
                if (low >= 0xdc00 and low <= 0xdfff) {
                    const units = [_]u16{ @intCast(code), @intCast(low) };
                    const scalar = std.unicode.utf16DecodeSurrogatePair(&units) catch unreachable;
                    var bytes: [4]u8 = undefined;
                    const count = std.unicode.utf8Encode(scalar, &bytes) catch unreachable;
                    try std.json.Stringify.encodeJsonStringChars(bytes[0..count], options, out);
                    index += 3;
                    paired = true;
                }
            }
            if (!paired) try out.print("\\u{x:0>4}", .{@as(u16, @intCast(code))});
            index += n;
            start = index;
        } else index += n;
    }
    try std.json.Stringify.encodeJsonStringChars(text[start..], options, out);
}
pub fn write(writer: *std.json.Stringify, text: []const u8) std.json.Stringify.Error!void {
    try writer.beginWriteRaw();
    try encode(text, writer.options, writer.writer);
    writer.endWriteRaw();
}
pub fn field(writer: *std.json.Stringify, text: []const u8) std.json.Stringify.Error!void {
    try writer.beginObjectFieldRaw();
    try encode(text, writer.options, writer.writer);
    writer.endObjectFieldRaw();
}
