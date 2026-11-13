# Core contract test mapping

Execution receipt: the combined CMS integration suite passed 97 tests with zero failures (`/private/tmp/cms-wp21-tests.log`); all 12 compile-fail doctests passed (`/private/tmp/cms-wp21-doctests.log`). This supersedes authored-only/pending execution wording below for included tests, but does not close the explicitly uncovered source obligations.

Scope: WP21 core-contract assignment, inventory `scratch/plans/contentmgmt2-rust-effectful/SOURCE_INVENTORY.md` §6. Source root: `/Users/chiefkemist/Documents/OGTECH_Workspace/outcastgeek.com/library/platforms/contentmgmt2`. The integration entry is `core_contracts.rs`; modules below live in `tests/core_contracts/`. This document records authored behavior coverage, not passing execution. The primary implementor alone runs Cargo. No production source changes are part of this batch.

Names below refer to executable Rust tests, not source-string searches. Native effects run under the existing effect runtime through `futures_executor::block_on(program.run(Arc::new(())))`; no alternate runtime/backend is introduced.

## Assigned source functions

| Go source file / exact function | Rust behavior coverage |
|---|---|
| `definition_builders_test.go::TestNewFeaturedResourceListDefinition` | `definitions_temporal::definition_builders_preserve_order_metadata_and_unified_routes`: layout, featured/secondary/grid order, retained meta label. |
| `definition_builders_test.go::TestNewTitledResourceFormSection` | Same test: title/description keys and contained field. |
| `definition_builders_test.go::TestNewResultsResourceListDefinition` | Same test: results layout and section. |
| `registry_alias_compatibility_test.go::TestDefinitionRegistryRetainsNestedReferences` | `registry_retains_nested_aliases_but_copies_outer_values_and_duplicate_order`: caller-to-lookup and lookup-to-caller filter and route-map edits. |
| `registry_alias_compatibility_test.go::TestDefinitionRegistryPreservesDuplicateOrderAndOuterSlice` | Same test: registration order, last duplicate definition, detached outer vector, unknown-kind refusal. |
| `registry_alias_compatibility_test.go::TestDefinitionQueryObservesPageCallbackMutation` | `workflows::list_query_admits_repeated_filters_after_page_callback_and_reuses_description`: mutation inside page callback changes the default read by the same admission; deferred invocation and repeated runs. |
| `management_routing_test.go::TestUnifiedManagementRouteModeNormalizesListRoute` | `definition_builders_preserve_order_metadata_and_unified_routes`: dashboard becomes list for unified mode. Other functions belong to primary's dashboards/routing scope. |
| `datetime_helpers_test.go::TestNormalizeDateTimeFormValuesClassifiesInput` | `datetime_classification_translation_and_dst_use_declared_zone`: required, invalid zone, invalid date, summer UTC conversion, added winter offset case. |
| `datetime_helpers_test.go::TestNormalizeDateTimeFormValuesWithTranslatedMessage` | Same test: caller key translation and invalid-zone precedence. |
| `datetime_helpers_test.go::TestNormalizeDateTimeFormValuesWithTranslatedMessageRequiresCallerKeys` | Same test: no message without caller-provided key. |
| `datetime_helpers_test.go::TestFormatLocalizedDateTimeCtxUsesTimeZoneAndLocaleFormatting` | `temporal_localized_formats_and_duration_precedence_match_source`: French month/date/time/joiner and Toronto EDT. |
| `datetime_helpers_test.go::TestLocalDateInTimeZoneUsesConfiguredTimeZone` | `datetime_classification_translation_and_dst_use_declared_zone`: UTC instant projects onto previous local date and exact civil fields. |
| `datetime_helpers_test.go::TestTemporalStatusAtUsesConfiguredTimeZone` | Same test: today/upcoming/past against supplied instant. |
| `duration_helpers_test.go::TestRequestedDurationFormValueReturnsCanonicalOrRawFallback` | `requested_duration_keeps_canonical_or_raw_fallback_from_current_decoded_form`: typed 2h becomes 120; invalid typed input returns raw PT30M; invalid text retained. Tests use decoded native form, not an ambient request. |
| `duration_helpers_test.go::TestNormalizeDurationFormValuesPrefersTypedLocalizedText` | `temporal_localized_formats_and_duration_precedence_match_source`: French multiweek text. |
| `duration_helpers_test.go::TestNormalizeDurationFormValuesSupportsStructuredMultiWeekParts` | Same test: weeks/days/hours/minutes canonical 24750. |
| `duration_helpers_test.go::TestNormalizeDurationFormValuesRejectsInvalidTypedTextEvenWhenCanonicalIsPresent` | Same test: invalid typed input refuses hidden canonical fallback. |
| `duration_helpers_test.go::TestNormalizeDurationFormValuesUsesHiddenCanonicalWhenTypedAndStructuredValuesAreEmpty` | Same test: hidden canonical accepted only when edits absent. |
| `duration_helpers_test.go::TestFormatDurationCtxFormatsCanonicalAndLegacyDurations` | Same test: canonical 90, legacy 1.5h and multiweek French output. |
| `contract_payload_mapper_test.go::TestContractPayloadMapperBuildsLocalizedMapsFromConfiguredInputs` | `content::payload_mapper_normalizes_languages_and_completeness_requires_every_language`: normalized language maps, format bodies, descriptions, complete/incomplete required-language set and detached key projection. |
| `contract_payload_mapper_test.go::TestContractLanguageNormalizerPreservesConfiguredLocalesAndFallsBack` | `language_normalization_preserves_configured_locales_and_field_suffixes`: all six exact source examples plus field suffix normalization. |
| `content_format_test.go::TestContentFormatFromStoredFields` | `formats_select_storage_and_contract_body_without_losing_source`: markdown/org precedence and EDN prefix. |
| `content_format_test.go::TestContentStorageColumnsForFormat` | Same test: EDN markdown prefix/suppression and org exclusive column. |
| `content_format_test.go::TestContractContentBodyFromLocalizedDraft` | Same test: org body and absent markdown/EDN. |
| `content_helpers_test.go::TestSanitizeHTMLPreservesRichContentAttributes` | `sanitizer_preserves_authored_rich_attributes_and_removes_handlers`: exact rich attributes retained; onclick/onerror removed. |
| `content_assets_test.go::TestContentAssetURLResolvesConfiguredCDN` | `asset_urls_use_explicit_cdn_then_public_base_and_keep_absolute_inputs`: explicit native config replaces process-global env mutation. |
| `content_assets_test.go::TestAbsoluteContentAssetURLUsesRequestBaseWhenCDNUnset` | Same test: caller supplies public origin; relative URL joins with it. The source request base is the origin, not its request path. |
| `content_assets_test.go::TestAbsoluteContentAssetURLPreservesAbsoluteInputs` | Same test: absolute media input preserved; blank input stays blank. |
| `content_link_shortcodes_test.go::TestContentLinkShortcodeCatalogDelegatesProjectLookupInputs` | `link_shortcode_catalog_delegates_inputs_and_uses_generic_cache_key`: argument order, request label, language, presentation resolver count, rendered link/cache key, lazy callback and blank-value no-lookup. |
| `content_link_shortcodes_test.go::TestLocalizedContentLinkShortcodeResolverSelectsRequestedLanguageRow` | `localized_content_link_selects_requested_row_and_falls_back_to_first_row`: exact French row/href/label/cache key plus missing-language first-row fallback. |
| `content_link_shortcodes_test.go::TestContentLinkTargetResolverBuildsTargetFromLookup` | `content_link_target_maps_lookup_and_retains_absence`: supplied row projects label/href; absent row stays unresolved. |
| `resource_helpers_test.go::TestResourceFilterURL` | `workflows::filter_toggle_and_search_token_contracts_preserve_encoding_order_and_limits`: three source URL cases. |
| `resource_helpers_test.go::TestSlugFilename` | `delimited_json_and_filename_helpers_keep_source_order_and_fallbacks`: normal extension and blank-slug fallback. |
| `resource_helpers_test.go::TestSplitDelimitedValues` | Same test: exact comma/newline/semicolon order. |
| `resource_helpers_test.go::TestJSONTextObjectHelpers` | Same test: trimmed encoding/decoding and invalid JSON empty fallback. |
| `resource_helpers_test.go::TestDelimitedJSONTextPairs` | Same test: ordered pair array plus empty array. |
| `resource_helpers_test.go::TestJSONTextPairValues` | Same test: per-field nonblank newline projection. |
| `filter_query_test.go::TestDefinitionListQueryParsesRepeatedMultiValueFilters` | `list_query_admits_repeated_filters_after_page_callback_and_reuses_description`: page/page size, dedup/sort ffi/rust, compatibility first value and filtered state. Status default also exercises callback mutation. |
| `filter_query_test.go::TestResourceFilterToggleURLAddsAndRemovesRepeatedValues` | `filter_toggle_and_search_token_contracts_preserve_encoding_order_and_limits`: exact add/remove URLs, preserved search, cleared page. |
| `search_tokens_test.go::TestSearchTokensArePunctuationTolerantAndDeduplicated` | Same test: punctuation, lowercase, first-seen dedup, accented letter and numeric token. |
| `search_tokens_test.go::TestSearchTokensHonorsConfiguredLimit` | Same test: two-token cap. |
| `access_guards_test.go::TestPrincipalHasAnyRoleMatchesConfiguredRoles` | `access_public_bypass_skips_resolver_and_principal_rules_keep_configured_roles`: trim, matching role, empty configured list, case sensitivity. A nil borrowed principal is unrepresentable. |
| `access_guards_test.go::TestRequireManagedPrincipalAccessSkipsPublicSurface` | Same test: public disposition proceeds, bypass flag, resolver never invoked. |
| `access_guards_test.go::TestRequireManagedPrincipalAccessRedirectsUnauthenticatedManager` | `access_redirect_forbidden_and_unauthorized_publish_exact_dispositions`: denied disposition, finite 302 response and /login location. |
| `access_guards_test.go::TestRequireManagedPrincipalAccessRendersForbidden` | Same test: MEMBER denied by ADMIN rule, 403 and exact Forbidden body. |
| `access_guards_test.go::TestRequirePrincipalAccessUsesUnauthorizedWhenRedirectDisabled` | Same test: no redirect, 401. |
| `access_guards_test.go::TestRequirePrincipalAccessAllowsAuthenticatedPrincipalByDefault` | `access_public_bypass_skips_resolver_and_principal_rules_keep_configured_roles`: explicit principal admitted without rule or resolver call. |
| `workflow_compatibility_test.go::TestMutationPreservesValidationBeforeAndCommandOrder` | `mutation_runs_validation_before_command_and_stops_on_before_failure`: exact validate/before/command trace, validation and before mutations visible in executor, before cause retained and command skipped. |
| `workflow_compatibility_test.go::TestValidationResultValidatorErrorTakesPrecedence` | `validation_result_error_takes_precedence_over_supported_and_refused_actions`: both supported create and refused update publish no input and original result-validator cause. |
| `binder_compatibility_test.go::TestBindingCollectionsObserveLaterBinderChanges` | `binder_collections_keep_live_maps_and_ordered_callback_replacement` covers validation index callback order/replacement state and live shared map changes. `command_execution_uses_mutated_identity_live_bindings_and_result_map_aliases` covers command bindings registered after service construction. Exact replacement of a later element in the Go command-binder variadic slice is not demonstrated; native public collection takes immutable borrowed trait references. See unmet obligations. |
| `resource_flow_helpers_test.go::TestBuildCommandInputPreservesFormParsing` | `command_input_form_parser_trims_values_rejects_malformed_and_honors_initialized_form`: malformed percent escape, trimmed body-before-query value, authoritative initialized form despite malformed current body. |
| `cue_caller_compatibility_test.go::TestCallerCueValidatorResolvesPublicCaller` | Native replacement: `native_contract_routes_caller_selector_and_preserves_setup_error_identity` verifies explicit builtin/project selector routing with real native codecs, setup error pointer identity and absent validator behavior. Stack-based public-caller discovery is intentionally not recreated. |
| `cue_caller_compatibility_test.go::TestCallerCueValidatorExtremeSkipMatchesLegacy` | Native API has no stack skip argument. Test above exercises explicit caller selection and missing selector/validator outcomes. Legacy skip arithmetic has no native behavior to test. |
| `cue_validation_test.go::TestCueValidationIssuesTrimsSelectorPrefix` | `native_contract_reports_selector_relative_field_path_and_rejects_unknown_or_malformed_values`: mutation result RedirectRoute violation is selector-relative; malformed serialized document, unknown selector and invalid action rejected. |
| `relation_service_outcomes_test.go::TestRelationServicesRejectNilAndUnconstructedInstances` | `relations::relation_zero_services_publish_nothing_and_factories_refuse_unbound_capabilities`: all three zero services publish None; all three constructors reject zero bindings. Required registry/callback refs cannot be nil. |
| `relation_service_outcomes_test.go::TestRelationOwnerFactoriesPreservePublicationAndOneShotClaim` | `relation_lookup_is_lazy_reusable_and_publishes_hydrated_selection` and `relation_validation_detaches_rows_and_mutation_consumes_claim_once`: invalid requests skip callbacks, source registry/descriptor/typed selection, hydrated selection publication, check/application order counts, detached projection, before/write counts, revision and replay rejection. |
| `internal/queries/detail_compatibility_test.go::TestDetailServiceNilAndZeroValuesAdmitDefaultQuery` | `default_detail_service_admits_query_values_without_registry_or_language`: zero native service admits id/slug, absent parent/locale; no nil receiver in Rust. |
| `internal/commands/internal/transition_test.go::TestActionResolutionRejectsReusedAndForgedStages` | Public behavior: `command_action_availability_is_ordered_reusable_and_appends_translation_once` and `command_missing_executor_and_refused_action_never_invoke_result_validator`; private-stage forgery/move negative obligation remains below. |
| `internal/queries/internal/transition_test.go::TestListAdmissionRejectsReusedAndForgedStages` | Public behavior: list admission is lazy/reusable with each run observing callback mutation. Private Go evidence factory replay is absent in native API; see below. |
| `internal/queries/internal/transition_test.go::TestDetailAdmissionRejectsReusedAndForgedStages` | Public behavior: default detail admission. Private-stage negative obligation below. |
| `internal/validation/internal/transition_test.go::TestActionValidationRejectsReusedAndForgedStages` | Public behavior: result-validator refusal precedence and mutation sequencing. Private-stage negative obligation below. |
| `internal/mutations/internal/transition_test.go::TestMutationRejectsReusedAndForgedStages` | Public behavior: mutation sequencing/failure; relation mutation has an actual reusable handle claim and its replay rejection is tested. Generic private-stage negative obligation below. |
| `internal/access/internal/access_outcome_test.go::TestAccessDispositionPublicationAndReuse` | Public allowed/bypassed/denied observation and consuming into_response exercised. Native Reply and into_response consume the outcome; denied double publication cannot be spelled safely. Compile-negative proof is not authored. |

Additional native cases: `shared_list_preserves_view_lengths_and_reallocation_independence` verifies independent lengths, overlapping append and separation after reallocation. `localized_collection_distinguishes_absent_values_and_preserves_raw_content_bytes` verifies absent-request normalizer suppression, present-empty normalization and exact malformed UTF-8 body bytes from rows. `relation_descriptor_and_selection_reject_wrong_identity_key_shape_and_operations` verifies invalid cardinality, wrong scalar key, missing keys, zero values and operation support. `command_execution_uses_mutated_identity_live_bindings_and_result_map_aliases` verifies input validator changes choose the executor, result identity defaults precede result validation, result-map edits survive while scalar replacements remain local, deferred execution and reusable descriptions. `command_missing_executor_and_refused_action_never_invoke_result_validator` covers both refusal branches.

## Explicit unmet or differently proved obligations

- Private generic Go stage evidence/forgery checks (commands, queries list/detail, validation, mutations) have no public Rust entry points. Public sequencing, failure and reusable-description tests above do not prove every ownership restriction. No compile-fail harness or in-module private tests have been authored in this integration-test-only assignment. These remain unverified negative type-contract obligations, not passing tests.
- Access double-denial publication is prevented by consuming native `AccessOutcome::into_response(self)` and non-Clone Reply. Public response values are tested; a compile-negative double-move proof remains unverified.
- Go command binder source-slice replacement is not tested exactly. Native immutable trait-reference slice does not expose mutation of a later slice slot. The tests verify supported late registration through the shared map and validation index-based collection's live callback state. Do not report the exact Go slice test fully covered.
- `architecture_relation_validation_test.go::{TestRelationAffectedOwnersRemainInFrozenTopology,TestRelationContractsPromoteThroughExistingOwners,TestRelationWorkflowsUseTypedPrerequisites,TestRelationValidatedValuesHavePrivateRepresentation,TestRelationWorkflowContractsMatchSource,TestRelationWorkflowValidatorRejectsRawPrerequisites}` are source topology/scanner tests. Per the task, no architecture scanner is restored. Actual typed descriptor/selection/claim behavior is tested; exhaustive graph/signature/private-representation and compile-negative raw-prerequisite assertions remain unverified.
- `architecture_workflows_execution_test.go::TestW15StateOperationMatrix` checks frozen Go graph table completeness. Tests above exercise actual native access branches instead; no native graph-matrix surrogate was authored. The frozen graph-table completeness obligation is not claimed passed.
- `architecture_relation_contracts_test.go`, `architecture_topology_manifest_test.go`, `architecture_workflows_foundation_test.go` and non-W15 portions of `architecture_workflows_execution_test.go` are data records, not executable behavior tests. Their assigned core records guide typed behavioral tests above. No string scanner or copied Go topology fixture is introduced. `architecture_placement_test.go::TestOwnerAndInterfacePlacement` is likewise not recreated.
- Transport reads for localized request helpers and duration helpers are not exercised here; decoded public value contracts are exercised. These additional localization/duration transport obligations remain unverified.

## Other-owner coordination

The following are excluded from this core assignment and belong to the other WP21 owners; this file does not claim they pass:

- `definition_runtime_factory_test.go::TestDefinitionRuntimeFactoryBuildsReusableServices` (composition root).
- `list_query_compatibility_test.go::TestListPageQueryUsesPublicListQuery` (rendering wrapper; direct query behavior is covered here).
- `filter_query_test.go::{TestWriteResourcePaginatedComponentStreamPreservesRepeatedTags,TestResourceFilteredCanonicalAndHreflangsPreserveRepeatedTags,TestResourceFilteredListSEOUsesTranslatedPatterns}` (stream rendering and SEO).
- `cue_validation_test.go::TestResourceDetailPageCueValidationAcceptsExtrasLinks` (detail rendering contract).
- `resource_helpers_test.go::TestResourceFreshnessKindAt` (rendering presentation).
- `resource_flow_helpers_test.go::{TestBuildResourceDraftRenderStateUsesGenericCommandAndMutationFlow,TestResolveSlugUpdateTargetRejectsMissingMetadata,TestResolveParentSlugUpdateTargetLoadsRows}` and `invalid_request_compatibility_test.go::TestDraftHelpersReadCurrentInvalidRequest` (forms).
- Remaining media/forms/authoring, rendering/resourcepages/dashboard, SEO/routing/platform files and their internal transition tests remain assigned elsewhere. None is silently counted as core coverage.

## Negative-contract handoff update

`core_contracts/type_contracts.rs` now contains twelve standard Rustdoc `compile_fail` cases using actual native types: moved access outcome (E0382), private access state (E0616), private command/list/detail implementation modules (E0603), private accepted validation fields (E0451), private ready mutation stage (E0603), private applied mutation fields (E0451), private relation descriptor/selection/validated-row payloads (E0451), and rejection of raw row Vec at typed relation mutation input (E0308). These supplement the earlier unmet-negative list; they are authored and pending primary integration/execution, not unverified missing snippets. No external compile-test framework or source scanner is used.

Primary integration instruction (production file outside this worker's permitted edit paths): add to `src/lib.rs`:

```rust
#[cfg(doctest)]
#[path = "../tests/core_contracts/type_contracts.rs"]
mod core_type_contracts;
```

The primary may then run ordinary crate doctests. The module-private cases prove non-entry through private implementation paths; they do not recreate Go intermediate stage objects that the pure Rust list/detail/command admission API does not publish. Native public reusable descriptions are tested separately. Generic mutation ready-stage privacy and accepted/applied publication privacy are exercised directly by the negative cases. Graph-table/scanner completeness, exact mutable command-binder slice replacement, and extra localized/duration transport-reader execution remain the precise uncovered obligations described above. No claim of passing tests is made until the sole runner reports results.
