# RPC

An `RpcOperation<Q, A, E>` pairs stable operation identity and version with distinct request, success and typed domain-error codecs. `RpcHandler` keeps the typed pairing through in-process calls. Encoded calls use bounded frames and a versioned `RpcEnvelope`; malformed framing, schema failure, unknown operations, typed domain failures and transport/backend failures remain distinguishable.

An executable unary or streaming protocol requires its first typed handler at construction: `RpcProtocol::from_handler(policy, handler)?` or `RpcStreamProtocol::from_handler(policy, handler)?`. `rpc_protocol!` and `rpc_stream_protocol!` apply this rule and register further handlers in source order. There is no public empty executable protocol constructor.

Streaming requires an acknowledgement before the server pulls another item. After chunk sequence `u64::MAX` is acknowledged, a terminal result is still valid; another chunk fails with `SequenceExhausted`.

For prepared unary calls, `rpc_operation!` accepts `prepared_codec:` along with
the stable `id`, `version` and `limits`. `PreparedRpcMemory` provisions pending
call records and both complete frame buffers. Each call checks those buffers
against the configured wire limits and acquires the codec workspaces and
`PreparedRpcChannel` reservation before the request codec runs. The provider
owns the actual exchange until it settles and fills the admitted response
through bounded appends. `PreparedInProcessRpcChannel` uses those same frames
and reserves its request reader and response writer as part of admission.

`PreparedRpcClient::call_in` uses the enclosing prepared future's cancellation
registration. A pre-dispatch memory refusal releases the reservation without
invoking the transport exchange.
`ResponseMemory` carries the call ID when a reply arrives but its typed
response cannot fit the selected decode storage. Transport failure remains
uncertain once dispatch begins. When sharing wire pools between client and
in-process channel, size them for both ends of each simultaneous call. The
existing streaming interfaces retain their transport and acknowledgement
contracts.

The caller supplies frame and JSON resource limits in `RpcEnvelopePolicy`. `EntityDispatcher::from_handler` (or `from_addressed_handler`) requires the first typed handler; `entity! { policy: ..., entity_type: ..., handler: ... }` is the macro form, while `entity! { dispatcher: ..., ... }` adds to an existing dispatcher. An addressed registration validates the decoded request against the target entity before calling business logic. Its reply is encoded as a success or domain-error envelope and saved by the Cluster message store only after the runner's completion phase.

See the [provider-neutral example](../../tests/fixtures/shared/provider_program.rs),
`rpc_cpu` and `entity_macro`, for `rpc_operation!` and `rpc_protocol!` definitions
with the CPU provider supplied through `R`.
