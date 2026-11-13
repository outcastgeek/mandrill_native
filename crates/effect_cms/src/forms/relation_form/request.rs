//! The relation request action.
//!
//! The action is the existing browser transport for the relation control: it
//! prevents the default submit, guards against a pending request, preserves the
//! focused element, rebuilds the target from the bound address and the selected
//! values, removes the transport of rejected options, applies the search value,
//! marks the root pending and busy, announces the localized loading text, awaits the
//! request, announces the localized failure text on error, and restores pending
//! state and focus in every case.
//!
//! Values are inserted as quoted string literals; the action text itself is a
//! `data-on` expression, not markup.

/// Build the relation request action.
#[must_use]
pub fn relation_request_action(
    id: &str,
    href: &str,
    selected_param: &str,
    search_param: &str,
    loading: &str,
    failed: &str,
) -> String {
    let quote = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    let mut action = String::new();
    action.push_str("(async () => { evt.preventDefault(); const root = document.getElementById(");
    action.push_str(&quote(id));
    action.push_str("); if (!root || root.dataset.pending === 'true') return; const activeID = document.activeElement?.id; const focusID = activeID && root.contains(document.activeElement) ? activeID : evt.currentTarget.id; const target = new URL(");
    action.push_str(&quote(href));
    action.push_str(", window.location.origin); const selection = document.getElementById(");
    action.push_str(&quote(&format!("{id}-selection")));
    action.push_str("); target.searchParams.delete(");
    action.push_str(&quote(selected_param));
    action.push_str("); if (selection && selection.selectedOptions.length) { for (const option of selection.selectedOptions) target.searchParams.append(");
    action.push_str(&quote(selected_param));
    action.push_str(", option.value); } else { target.searchParams.append(");
    action.push_str(&quote(selected_param));
    action.push_str(", ''); } if (selection) { target.searchParams.delete(selection.name + \".rejected\"); target.searchParams.delete(selection.name + \".rejected_keys\"); } const rejected = root.querySelectorAll(\"input[data-relation-rejected-transport]\"); const rejectedNames = new Set(); for (const field of rejected) rejectedNames.add(field.name); for (const name of rejectedNames) target.searchParams.delete(name); for (const field of rejected) target.searchParams.append(field.name, field.value); const search = document.getElementById(");
    action.push_str(&quote(&format!("{id}-search")));
    action.push_str("); if (search) target.searchParams.set(");
    action.push_str(&quote(search_param));
    action.push_str(", search.value); root.dataset.pending = 'true'; root.setAttribute('aria-busy', 'true'); const status = document.getElementById(");
    action.push_str(&quote(&format!("{id}-status")));
    action.push_str("); if (status) status.textContent = ");
    action.push_str(&quote(loading));
    action.push_str("; try { await @get(target.toString()); } catch { const failedStatus = document.getElementById(");
    action.push_str(&quote(&format!("{id}-status")));
    action.push_str("); if (failedStatus) failedStatus.textContent = ");
    action.push_str(&quote(failed));
    action.push_str("; } finally { const currentRoot = document.getElementById(");
    action.push_str(&quote(id));
    action.push_str("); if (currentRoot) { delete currentRoot.dataset.pending; currentRoot.removeAttribute('aria-busy'); } const focus = document.getElementById(focusID) || document.getElementById(");
    action.push_str(&quote(&format!("{id}-previous")));
    action.push_str(") || document.getElementById(");
    action.push_str(&quote(&format!("{id}-next")));
    action.push_str(") || document.getElementById(");
    action.push_str(&quote(&format!("{id}-selection")));
    action.push_str("); focus?.focus({preventScroll:true}); } })()");
    action
}
