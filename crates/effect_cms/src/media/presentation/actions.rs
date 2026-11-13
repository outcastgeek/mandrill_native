//! Media Datastar action strings.

/// Escape a value for a single-quoted JavaScript string literal.
#[must_use]
pub fn js_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "")
}

/// Lock page scrolling for an overlay.
#[must_use]
pub fn media_scroll_lock_action() -> String {
    "const html = document.documentElement; const body = document.body; if (html && body) { if (html.dataset.mediaScrollLock !== 'true') { html.dataset.mediaScrollLockPreviousOverflow = html.style.overflow || ''; body.dataset.mediaScrollLockPreviousOverflow = body.style.overflow || ''; } html.dataset.mediaScrollLock = 'true'; body.dataset.mediaScrollLock = 'true'; html.style.overflow = 'hidden'; body.style.overflow = 'hidden'; }".to_owned()
}

/// Unlock scrolling once no overlay remains.
#[must_use]
pub fn media_scroll_unlock_when_idle_action() -> String {
    "queueMicrotask(() => { const html = document.documentElement; const body = document.body; if (!html || !body || document.querySelector('.media-ui-overlay')) { return; } html.style.overflow = html.dataset.mediaScrollLockPreviousOverflow || ''; body.style.overflow = body.dataset.mediaScrollLockPreviousOverflow || ''; delete html.dataset.mediaScrollLock; delete body.dataset.mediaScrollLock; delete html.dataset.mediaScrollLockPreviousOverflow; delete body.dataset.mediaScrollLockPreviousOverflow; })".to_owned()
}

/// Clear a mount and unlock scrolling when idle.
#[must_use]
pub fn media_mount_clear_action(root_id: &str) -> String {
    let clear = media_mount_clear_only_action(root_id);
    if clear.is_empty() {
        return String::new();
    }
    format!("{clear}; {}", media_scroll_unlock_when_idle_action())
}

/// Clear a mount without the scroll unlock.
#[must_use]
pub fn media_mount_clear_only_action(root_id: &str) -> String {
    let root_id = root_id.trim();
    if root_id.is_empty() {
        return String::new();
    }
    format!(
        "const mount = document.getElementById('{}'); if (mount) {{ mount.innerHTML = ''; }}",
        media_js_string(root_id)
    )
}

/// Set an optional signal false and clear the mount.
#[must_use]
pub fn media_dismiss_action(signal: &str, root_id: &str) -> String {
    let signal = signal.trim();
    let base = if signal.is_empty() {
        String::new()
    } else {
        format!("${signal} = false")
    };
    let clear = media_mount_clear_action(root_id);
    if base.is_empty() {
        return clear;
    }
    if clear.is_empty() {
        return base;
    }
    format!("{base}; {clear}")
}

/// Set a picker signal false and clear the picker mount.
#[must_use]
pub fn media_picker_dismiss_action(signal: &str, root_id: &str) -> String {
    media_dismiss_action(signal, root_id)
}

/// Clear the picker mount.
#[must_use]
pub fn media_picker_mount_clear_action(root_id: &str) -> String {
    media_mount_clear_action(root_id)
}

/// Escape a value for the generated inline JavaScript.
#[must_use]
pub fn media_js_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}
