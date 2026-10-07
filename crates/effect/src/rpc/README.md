# RPC

An `RpcOperation<Q, A, E>` pairs stable operation identity and version with distinct request, success and typed domain-error codecs. `RpcHandler` keeps the typed pairing through in-process calls. Encoded calls use bounded frames and a versioned `RpcEnvelope`; malformed framing, schema failure, unknown operations, typed domain failures and transport/backend failures remain distinguishable.

An executable unary or streaming protocol requires its first typed handler at construction: `RpcProtocol::from_handler(policy, handler)?` or `RpcStreamProtocol::from_handler(policy, handler)?`. `rpc_protocol!` and `rpc_stream_protocol!` apply this rule and register further handlers in source order. There is no public empty executable protocol constructor.

Streaming requires an acknowledgement before the server pulls another item. After chunk sequence `u64::MAX` is acknowledged, a terminal result is still valid; another chunk fails with `SequenceExhausted`.

The caller supplies frame and JSON resource limits in `RpcEnvelopePolicy`. `EntityDispatcher::from_handler` (or `from_addressed_handler`) requires the first typed handler; `entity! { policy: ..., entity_type: ..., handler: ... }` is the macro form, while `entity! { dispatcher: ..., ... }` adds to an existing dispatcher. An addressed registration validates the decoded request against the target entity before calling business logic. Its reply is encoded as a success or domain-error envelope and saved by the Cluster message store only after the runner's completion phase.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`rpc_cpu` and `entity_macro`, for `rpc_operation!` and `rpc_protocol!` definitions
with the CPU provider supplied through `R`.
