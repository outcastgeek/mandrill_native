# Media/forms/authoring source behavior coverage

Execution receipt: the combined CMS integration suite passed 97 tests with zero failures (`/private/tmp/cms-wp21-tests.log`); all 12 compile-fail doctests passed (`/private/tmp/cms-wp21-doctests.log`). This supersedes authored-only/pending execution wording below for included tests, but does not close the explicitly uncovered source obligations.

WP21 authored regression ledger. Implementation helper ran no Cargo/build/test/check command. The primary runner owns results. A named test is an authored behavior assertion, not a claimed passing result. Partial rows explicitly retain their remaining obligations.

All tests are in `media_forms.rs` and `media_forms/{assets,render,forms,authoring,flows,experience}.rs`. Native HTTP authoring cases use the shared real `tests/support/native_http.rs` harness, exercising response status, headers and body; concurrency uses a two-target async barrier; timeout/cancellation verify dropped work; no source-text assertion tests exist.

## Per-source test-function mapping

| Source file/function | Authored Rust behavior or explicit unmet obligation |
| --- | --- |
| `media_asset_form_page_test.go` / `TestMediaAssetFormPageBuildsCreateFormFromResourceDefinition` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_form_page_test.go` / `TestMediaAssetFormPageBuilderBuildsEditFormFromStoredAsset` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_presentation_views_test.go` / `TestBuildResourcePreviewImageMapSkipsBlankKeys` | assets::preview_map_skips_blank_keys_and_trims_both_projections |
| `media_presentation_views_test.go` / `TestBuildContentMediaCollectionViewUsesHeroRoleAndSkipsEmbeddedAssetsFromSlides` | assets::content_attachment_video_provider_normalizes_and_collection_excludes_embedded |
| `media_presentation_views_test.go` / `TestRenderContentMediaCollectionHTMLComposesHeroAndSlideDeck` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_presentation_views_test.go` / `TestRenderContentVideoMediaHTMLFallsBackToHeroImage` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_delete_compatibility_test.go` / `TestMediaDeleteUsesDirectStoreAndPreservesErrors` | flows::direct_media_delete_skips_read_and_preserves_store_error_identity (direct-store/no-read/error identity covered; missing-store/missing-id branches remain unmet) |
| `media_components_test.go` / `TestMediaDashboardRowActionsIdentifyTheirItem` | Primary resourcepages renderer lane exclusion: generic resourcepages compositions/management form are owned by primary. |
| `media_components_test.go` / `TestMediaWorkbenchInlineActionsAreOptIn` | Primary resourcepages renderer lane exclusion: generic resourcepages compositions/management form are owned by primary. |
| `media_components_test.go` / `TestResourceListActionsIdentifyTheirItem` | Primary resourcepages renderer lane exclusion: generic resourcepages compositions/management form are owned by primary. |
| `media_components_test.go` / `TestMediaSlideDeckRendersSingleVideoCardWithVideoPlayer` | render::single_video_uses_player_before_prepared_shortcode_and_multiple_items_use_pager |
| `media_components_test.go` / `TestMediaSlideDeckRendersMultipleVideoItemsAsSlides` | render::single_video_uses_player_before_prepared_shortcode_and_multiple_items_use_pager |
| `media_components_test.go` / `TestMediaAttachmentManagerRendersAdditionalCountAndDragAttributes` | render::attachment_manager_preserves_drag_mutation_and_owner_form_protocol (protocol attrs; translated count formatting not yet asserted) |
| `media_components_test.go` / `TestMediaInlineCreateDrawerAcceptsPathInputAndUsesServerRenderedPreview` | render::inline_drawer_paths_use_server_preview_and_stable_form_target |
| `media_components_test.go` / `TestMediaPickerDialogContentLayersAboveOverlay` | render::picker_and_shortcode_content_layer_overlays_and_preserve_selection_forms |
| `media_components_test.go` / `TestMediaCreatePreservesPickerAndShortcodeMounts` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_components_test.go` / `TestMediaShortcodeDialogContentLayersAboveOverlay` | render::picker_and_shortcode_content_layer_overlays_and_preserve_selection_forms |
| `media_components_test.go` / `TestMediaLibraryPaginationSubmitsTheFilterFormExplicitly` | render::library_pagination_submits_explicit_filter_form_without_javascript_defaults |
| `media_components_test.go` / `TestMediaInlineCreatePreviewHelpersUseStableServerOwnedTargets` | render::inline_preview_helper_keeps_stored_path_and_server_resolves_preview |
| `media_dashboard_test.go` / `TestMediaDashboardRowBuildsTypedMediaRow` | flows::media_dashboard_row_and_page_urls_use_localized_action_and_provider_metadata |
| `media_dashboard_test.go` / `TestBuildMediaDashboardPageUsesLocalizedActionForFiltersAndPagination` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_records_test.go` / `TestMediaAssetPrimaryURLAndPreviewResolveRelativeAssetsThroughResolver` | assets::primary_preview_resolve_paths_and_preserve_explicit_cdn |
| `media_asset_records_test.go` / `TestMediaAssetPreviewURLPrefersRenderableThumbnailAndExplicitCDNURL` | assets::primary_preview_resolve_paths_and_preserve_explicit_cdn |
| `media_asset_records_test.go` / `TestMediaAssetPreviewURLHandlesEmptyAndNonRenderableAssets` | assets::primary_preview_resolve_paths_and_preserve_explicit_cdn |
| `media_store_helpers_test.go` / `TestPrepareMediaAssetRecordForCreateAppliesDefaults` | assets::creation_defaults_are_lazy_and_update_preserves_created_metadata |
| `media_store_helpers_test.go` / `TestPrepareMediaAttachmentStoreRecordsForReplaceIsResourceAgnostic` | assets::attachment_store_replace_skips_blank_and_retains_original_positions |
| `media_slide_helpers_test.go` / `TestRenderMediaSlideItemsUsesShortcodeMarkdownForResidualMedia` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceSearchPickerRedirectsDirectBrowserRequestsToMediaList` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceSearchPickerReturnsDatastarPatchWithMediaItems` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceSearchPickerSupportsSecondPage` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceSearchPickerSearchRendersResolvedPreviewForPathAsset` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceSearchPickerPaginationRendersResolvedPreviewForPathAsset` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceShortcodeModalReturnsLibraryMediaItemsWithoutAttachments` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceListSearchRendersResolvedPreviewForPathAsset` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceListPaginationRendersResolvedPreviewForPathAsset` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceBuildInlineCreatePropsKeepsStoredPathAndBuildsResolvedPreviewOnServer` | render::inline_preview_helper_keeps_stored_path_and_server_resolves_preview |
| `media_asset_experience_test.go` / `TestResourceMediaFormFrameUsesOverrideAttachments` | flows::media_form_frame_override_precedes_store_backed_attachment_loader (injected loader branch; actual attachment-store implementation integration remains unmet) |
| `media_asset_experience_test.go` / `TestResourceMediaFormFrameLoadsStoreBackedAttachments` | flows::media_form_frame_override_precedes_store_backed_attachment_loader (injected loader branch; actual attachment-store implementation integration remains unmet) |
| `media_asset_experience_test.go` / `TestMediaExperiencePreviewInlineCreateReturnsDatastarPatchWithResolvedImageURL` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `media_asset_experience_test.go` / `TestMediaExperienceCreateUpdateDeleteUseGenericCommandPipeline` | experience::media_experience_create_update_delete_use_generic_command_pipeline: actual MediaAssetService/controller, canonical generic validation+command services and late-bound media bindings, native POST form parsing and typed ID route captures, consumer store persistence across create/update/delete, original creation metadata, localized303 Location and command result validator ordering. |
| `media_asset_experience_test.go` / `TestMediaExperienceEditRendersStoredMediaAssetForm` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `content_media_models_test.go` / `TestContentMediaItemsFromAttachmentItemsNormalizesVideoProvider` | assets::content_attachment_video_provider_normalizes_and_collection_excludes_embedded |
| `inline_input_compatibility_test.go` / `TestInlineCreateRetainsInputIdentityAcrossCallbacks` | flows::inline_create_retains_shared_input_identity_through_validation_creation_and_error_render |
| `invalid_request_compatibility_test.go` / `TestDraftHelpersReadCurrentInvalidRequest` | forms::slug_select_failure_observes_current_invalid_error_and_parent_loader (slug callback-time error + successful parent; draft missing-build and parent changed-to-nil branch remain unmet) |
| `localized_tab_props_test.go` / `TestBuildLocalizedContentTabSetBuildsFieldNamesLabelsAndFormatHelper` | forms::localized_content_tabs_keep_normalized_names_labels_and_format_helper (language labels; translated-with-language-name variant remains unmet) |
| `localized_tab_props_test.go` / `TestNewLocalizedTabsPropsBuildsAuthoringAndMediaFromRuntime` | forms::runtime_builds_authoring_and_media_mounts_in_localized_props |
| `localized_tab_props_test.go` / `TestAttachValidationIssuesToFormSectionsMarksLocalizedLanguageTab` | forms::localized_validation_marks_only_target_language_tab |
| `localized_tabs_authoring_test.go` / `TestNewLocalizedTabsAuthoringPropsReflectsConfiguredAvailability` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `localized_tabs_authoring_test.go` / `TestLocalizedAuthoringFormAttributesMergeLocalizedSignalsIntoForm` | forms::authoring_widgets_merge_scoped_signals_without_submit_capture |
| `localized_tabs_authoring_test.go` / `TestLocalizedAuthoringFormAttributesForWidgetsMergeScopedSignals` | forms::authoring_widgets_merge_scoped_signals_without_submit_capture |
| `localized_tabs_authoring_test.go` / `TestLocalizedAutoTranslateRunButtonDelaysPostAfterPreparingWidget` | forms::localized_submit_guard_and_delayed_translation_retain_client_protocol |
| `localized_tabs_authoring_test.go` / `TestLocalizedSubmitControlAttributesReflectBlockedStateSemantically` | forms::localized_submit_guard_and_delayed_translation_retain_client_protocol |
| `submit_actions_test.go` / `TestRequestedContentStatusPrefersExplicitSubmitAction` | forms::explicit_submit_action_precedes_status_and_fallback |
| `submit_actions_test.go` / `TestRequestedContentStatusFallsBackToCurrentStatus` | forms::explicit_submit_action_precedes_status_and_fallback |
| `resource_flow_helpers_test.go` / `TestBuildResourceDraftRenderStateUsesGenericCommandAndMutationFlow` | forms::draft_render_state_parses_once_then_projects_row_drafts_media_and_extra |
| `resource_flow_helpers_test.go` / `TestResolveSlugUpdateTargetRejectsMissingMetadata` | forms::slug_select_failure_observes_current_invalid_error_and_parent_loader (slug callback-time error + successful parent; draft missing-build and parent changed-to-nil branch remain unmet) |
| `resource_flow_helpers_test.go` / `TestResolveParentSlugUpdateTargetLoadsRows` | forms::slug_select_failure_observes_current_invalid_error_and_parent_loader (slug callback-time error + successful parent; draft missing-build and parent changed-to-nil branch remain unmet) |
| `resource_flow_helpers_test.go` / `TestBuildCommandInputPreservesFormParsing` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `resource_form_actions_test.go` / `TestResourceFormActionPropsPreferPublicationActionsOverGenericSave` | forms::publication_actions_suppress_generic_save_and_non_submit_actions |
| `authoring_helpers_test.go` / `TestAuthoringTranslationFieldJSONUsesProviderShape` | authoring::provider_json_shape_fences_escape_repair_and_custom_fallback_signals |
| `authoring_helpers_test.go` / `TestParseAuthoringTranslationFieldsJSONTrimsFencesAndRepairsEscapes` | authoring::provider_json_shape_fences_escape_repair_and_custom_fallback_signals |
| `authoring_helpers_test.go` / `TestAuthoringTranslationFallbackSignalsJSONUsesProvidedSignals` | authoring::provider_json_shape_fences_escape_repair_and_custom_fallback_signals |
| `authoring_translation_runtime_test.go` / `TestDefaultAuthoringTranslationTimeoutIsSixtySeconds` | authoring::provider_json_shape_fences_escape_repair_and_custom_fallback_signals |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationReturnsSignalPatchForMultilineField` | authoring::localized_translation_delivers_native_multiline_signal_patch_and_unavailable_branch |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationReturnsUnavailableSignalPatchWhenTranslatorMissing` | authoring::localized_translation_delivers_native_multiline_signal_patch_and_unavailable_branch |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationRedirectsUnauthenticatedUsersToLocalizedLogin` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationRejectsUnauthorizedRolesWithHTMLError` | authoring::access_reply_precedes_translator_and_form_parsing (supplied forbidden access reply; configured role resolver integration remains unmet) |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationNormalizesTargetsAndReturnsFieldSignalPatches` | authoring::translation_normalizes_targets_and_runs_distinct_targets_concurrently |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationTranslatesTargetsInParallel` | authoring::translation_normalizes_targets_and_runs_distinct_targets_concurrently |
| `authoring_translation_runtime_test.go` / `TestLocalizedAuthoringTranslationTimesOutHungTranslator` | authoring::translator_deadline_cancels_hung_future_and_renders_failure_patch (native timeout argument/clock and dropped work; native API supplies Duration rather than source context deadline) |
| `authoring_translation_runtime_test.go` / `TestTranslateAuthoringFieldsWithTimeoutPassesDeadlineContext` | authoring::translator_deadline_cancels_hung_future_and_renders_failure_patch (native timeout argument/clock and dropped work; native API supplies Duration rather than source context deadline) |
| `authoring_translation_runtime_test.go` / `TestRuntimeStubAuthoringTranslatorRespectsContextCancellation` | UNMET source end-to-end obligation: no equivalent complete Rust behavior test authored in this batch. |
| `internal/authoring/translation_compatibility_test.go` / `TestTranslationCallbacksShareNormalizedFields` | authoring::concurrent_translation_callbacks_share_the_normalized_field_backing_store |
| `internal/authoring/translation_compatibility_test.go` / `TestTranslationRetainsReturnedResultForLaterCallbacks` | authoring::ready_patch_reads_returned_shared_result_after_language_normalizer_mutation |
| `internal/authoring/translation_compatibility_test.go` / `TestTranslationReturnsOnContextCancellation` | authoring::cancellation_interrupts_active_translation_and_releases_its_future |
| `internal/authoring/internal/transition_test.go` / `TestTranslationRejectsReusedAndForgedStages` | UNMET literal private replay/forgery fixture: native private stages are not public values; see private transition obligations below. Public callback-once and identity cases do not substitute for that fixture. |
| `internal/forms/internal/field_state_test.go` / `TestDisabledFieldStatePreservesDraftAndIssues` | forms::disabled_field_state_preserves_rejected_draft_issues_and_help |
| `internal/forms/internal/transition_test.go` / `TestFormActionsRejectReusedAndForgedStages` | UNMET literal private replay/forgery fixture: native private stages are not public values; see private transition obligations below. Public callback-once and identity cases do not substitute for that fixture. |
| `internal/media/internal/assets/internal/transition_test.go` / `TestAssetCreateRejectsReusedAndForgedStages` | UNMET literal private replay/forgery fixture: native private stages are not public values; see private transition obligations below. Public callback-once and identity cases do not substitute for that fixture. |
| `internal/media/internal/attachmentflow/internal/transition_test.go` / `TestAttachmentReplacementRejectsReusedAndForgedStages` | UNMET literal private replay/forgery fixture: native private stages are not public values; see private transition obligations below. Public callback-once and identity cases do not substitute for that fixture. |
| `internal/media/internal/flows/internal/inline_transition_test.go` / `TestInlineCreateRejectsReusedAndMismatchedWitnesses` | UNMET literal private replay/forgery fixture: native private stages are not public values; see private transition obligations below. Public callback-once and identity cases do not substitute for that fixture. |
| `internal/media/internal/runtime/internal/picker_transition_test.go` / `TestPickerListRejectsReusedAndMismatchedStages` | UNMET literal private replay/forgery fixture: native private stages are not public values; see private transition obligations below. Public callback-once and identity cases do not substitute for that fixture. |
| `internal/media/internal/presentation/internal/legacy_components_test.go` / `TestMediaSourceModeExposesSelectedState` | render::inline_drawer_paths_use_server_preview_and_stable_form_target (reactive selected marker only; explicit mode-button selected-value equality remains unmet) |
| `internal/media/internal/presentation/internal/legacy_components_test.go` / `TestMediaPickerReturnFocusRestoresTheRightSectionAndScroll` | render::return_focus_and_scroll_unlock_wait_until_overlay_removal |
| `internal/media/internal/presentation/internal/legacy_components_test.go` / `TestMediaScrollLockActionsLockDocumentAndRestoreAfterLastOverlay` | render::return_focus_and_scroll_unlock_wait_until_overlay_removal |
| `internal/media/internal/presentation/internal/legacy_components_test.go` / `TestDocumentSlideUsesThumbnailOnlyForImagePreview` | render::document_slide_and_shortcode_picker_never_use_document_as_image |
| `internal/media/internal/presentation/internal/legacy_components_test.go` / `TestShortcodePickerDoesNotUseDocumentURLAsImage` | render::document_slide_and_shortcode_picker_never_use_document_as_image |

## Additional native obligations covered

- Attachment repeated-column ordering, blank-row callback observation, fallback-name precedence: `assets::attachment_collection_orders_columns_and_preserves_callback_observations`.
- Attachment stable dedupe, single-hero promotion and removal: `assets::attachment_promote_dedupe_remove_preserve_one_hero_and_order`.
- Source asset-create private test observable duration alias, store mutation and returned store ID: `flows::create_store_mutation_retains_duration_alias_and_returns_store_assigned_identity`. This required the primary production duration representation fix; helper made no production edits.
- Lazy store preparation and draft construction assert callbacks have not run before execution and exactly once within one execution.
- Picker lower-level second-page patch callback uses admitted query and resolved preview: `flows::picker_second_page_preserves_query_and_resolved_path_in_datastar_patch`. Direct-browser request does not invoke page builder: `flows::picker_native_request_redirects_without_touching_builder`. These do not claim full MediaExperience controller coverage.

## Private transition obligations

Source transition functions assigned: authoring TranslationRejectsReusedAndForgedStages; forms FormActionsRejectReusedAndForgedStages; media assets AssetCreateRejectsReusedAndForgedStages; attachmentflow AttachmentReplacementRejectsReusedAndForgedStages; inline InlineCreateRejectsReusedAndMismatchedWitnesses; picker PickerListRejectsReusedAndMismatchedStages.

Native workflows do not publish those mutable stage/evidence objects. Integration tests cannot construct their private representations. Literal private replay, substituted-owner and forged-stage attempts remain untested here. No production visibility hook or fake source scan was introduced. The public tests establish observable lazy call timing, per-execution callback counts, Arc identity and source error/patch branches only. Unit-test handoff: if a native owner retains an internally reusable witness/stage, place a `#[cfg(test)]` child at that owner and assert the same witness is consumed once, a copied/mismatched owner cannot call the external store/translator twice, and the source-specific invalid-stage outcome is retained. No unapproved production mutation/test hook is included in this batch.

## Remaining source end-to-end scope

The per-function UNMET rows are authoritative; in particular full MediaExperience list/shortcode/edit controller wiring, media create/edit page builders, residual shortcode renderer delegation, collection HTML/video image-fallback adapter composition, authoring localized-login/role integration and runtime stub cancellation still need equivalent source-level integration coverage. Parent instructed urgent completion and no additional exploratory work; these are explicit open obligations, not passing coverage.

The availability global toggle test is unimplemented to avoid racing global state in the current parallel test binary; isolated serialized coverage is still required. No accessibility audit, remediation, redesign or added acceptance policy was performed.


## Authorized bounded media-pipeline batch: 2026-11-13

The three `experience` tests use the production `new_media_asset_service`, its
controller and canonical `DefinitionServiceSet` validation/command services. The
consumer-owned test store receives the real mutation records. Actual native HTTP
POST bodies supply create/update values; typed native routes own update/delete ID
captures. No private capture fabrication, production visibility hook, scanner or
backend was introduced. The common test-only HTTP harness now accepts method/form
body and an optional typed route; its earlier GET/SSE wrapper remains in use.

- `media_experience_create_update_delete_use_generic_command_pipeline` verifies all
  three store operations, persisted URL/title/kind/status, creation metadata retained
  on update, deletion,303 redirect Location and both generic input validator calls
  plus result validation after each successful store operation.
- `media_experience_rejects_missing_url_before_generic_executor_or_store_write`
  verifies the canonical media validation error reaches actual form rehydration and
  the production Maud form renderer. It retains the typed ValidationError behind the
  consumer error's source chain. No store operation or command-result validator runs
  and no success redirect is emitted.
- `media_experience_consumer_create_failure_never_publishes_success_or_mutation_result`
  verifies one failing consumer-create attempt yields500 with no persisted record,
  success redirect, result-validator publication or subsequent update/delete write.

Focused execution: `cargo test -p effect_cms --test media_forms experience::`,
exit0,3 passed (`/private/tmp/cms-wp21-experience.log`). Existing combined execution:
`cargo test -p effect_cms --tests`, exit0,100 regular tests passed (scaffold1,
composition21,core34,media/forms44), `/private/tmp/cms-wp21-window-tests.log`.
Both commands had explicit60-second process-group bounds. The earlier12 passing
compile-fail doctests remain recorded in `/private/tmp/cms-wp21-doctests.log`; no
production library source changed in this batch. The remaining UNMET/partial rows
above remain open; these results do not establish full WP21 or plan completion.
