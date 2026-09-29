//! Compile-time contract shared by typed protocol definitions. A schema can
//! expose a representation codec or the structural JSON/byte schema surface.
fn has(comptime T: type, comptime name: []const u8) bool {
    return switch (@typeInfo(T)) {
        .@"struct", .@"union", .@"enum", .@"opaque" => @hasDecl(T, name),
        else => false,
    };
}

/// Enforce the structural codec interface at comptime. A codec must expose
/// `Decoded` plus either a representation codec pair or a structural schema
/// decode/encode pair; incomplete implementations fail compilation.
/// Parameters:
/// - `C: comptime type`: Candidate codec or schema type whose declarations are
///   inspected at compile time.
/// Returns (`void`): No runtime value; successful specialization proves that
///   the required type members and codec entry points are present.
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
