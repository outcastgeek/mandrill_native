# Schema

`PureCodec<A, Encoded>` keeps decoding, encoding and encoded-side structure together. `SchemaAst` describes representation, not an unchecked constructor for `A`. Parsing requires a consumer-selected `ParsePolicy` with explicit byte, depth, node, collection and issue limits. Decode and encode return typed `SchemaIssue` values with paths; direct type-side validation may be supplied when constructing an encoded value would be unnecessary.

The `schema!` procedural macro derives a typed record or tagged-enum codec from application fields and their codecs. Builders reject incomplete or duplicate definitions. Recursive codecs use weak schema references so a dropped schema graph does not retain itself. Representation export reports unsupported or lossy structures explicitly rather than silently changing validation semantics.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`schema_cpu`, for a `schema!` record and an effectful codec using the CPU
provider supplied through `R`.
