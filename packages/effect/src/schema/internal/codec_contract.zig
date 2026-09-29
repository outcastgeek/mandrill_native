//! Compile-time contract shared by typed protocol definitions. A schema can
//! expose a representation codec or the structural JSON/byte schema surface.
fn has(comptime T: type, comptime name: []const u8) bool {
    return switch (@typeInfo(T)) {
        .@"struct", .@"union", .@"enum", .@"opaque" => @hasDecl(T, name),
        else => false,
    };
}

pub fn require(comptime C: type) void {
    if (!has(C, "Decoded"))
        @compileError("schema must expose its decoded application type");
    const representation_codec = has(C, "Representation") and
        has(C, "DecodingEnvironment") and has(C, "EncodingEnvironment") and
        ((has(C, "decodeDirect") and has(C, "encodeDirect")) or
            (has(C, "decode") and has(C, "encode")));
    const structural_schema = has(C, "encodedNode") and
        (has(C, "decodeDirect") or has(C, "decode")) and
        (has(C, "encodeBytesDirect") or has(C, "encodeBytes"));
    if (!representation_codec and !structural_schema)
        @compileError("schema must provide matching decode and encode operations");
}
