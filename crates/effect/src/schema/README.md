# Schema

`PureCodec<A, Encoded>` keeps decoding, encoding and encoded-side structure together. `SchemaAst` describes representation, not an unchecked constructor for `A`. Parsing requires a consumer-selected `ParsePolicy` with explicit byte, depth, node, collection and issue limits. Decode and encode return typed `SchemaIssue` values with paths; direct type-side validation may be supplied when constructing an encoded value would be unnecessary.

The `schema!` procedural macro derives a typed record or tagged-enum codec from application fields and their codecs. Builders reject incomplete or duplicate definitions. Recursive codecs use weak schema references so a dropped schema graph does not retain itself. Representation export reports unsupported or lossy structures explicitly rather than silently changing validation semantics.

The executable [feature tests](../../tests/new_features.rs) cover record and enum macro construction, tagged dispatch, string constraints and round trips. The [external typing fixture](../../tests/ui/new_schema.rs) compiles the macro from a separate consumer crate. The [schema campaign](../../tests/feature_campaigns.rs) covers recursive value round trips, parser byte boundaries, concurrent first-use preparation across independent environment types, and focused exact/approximate representation export with an explicit node-limit failure. Full parser-budget and representation conformance remain separate requirements.

The external [provider-neutral consumer](../../tests/fixtures/shared/provider_program.rs) builds a record with `schema!` in `schema_cpu` and runs an effectful codec with its CPU provider. Both the independent and mixed-provider consumer binaries execute that same definition.
