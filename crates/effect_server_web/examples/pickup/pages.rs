//! Small semantic HTML views; dynamic text is escaped before entering trusted markup.

/// One accessible shell; static script/styles use the native per-response CSP nonce.
fn page(title: &str, content: &str, nonce: &str) -> String {
    let script = include_str!("browser.js");
    format!(
        r##"<!doctype html>
<html lang="en-US"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title} — Neighborhood café</title>
<style nonce="{nonce}">
:root {{ color-scheme: light; }}
body {{ margin: 0; background: white; color: black; font: 1rem/1.6 system-ui, sans-serif; }}
main {{ max-width: 38rem; margin: auto; padding: 1.5rem; overflow-wrap: anywhere; }}
a {{ color: #000080; }}
a:visited {{ color: #000080; }}
label {{ display: block; margin-top: 1rem; }}
input,button {{ font: inherit; color: black; background: white; border: 2px solid black; border-radius: .25rem; padding: .65rem; box-sizing: border-box; min-height: 44px; }}
input {{ display: block; width: 100%; }}
button {{ margin-top: 1rem; cursor: pointer; }}
button[aria-disabled=true] {{ color: black; border-style: dashed; cursor: default; }}
input[type=checkbox] {{ display: inline-block; width: 44px; height: 44px; vertical-align: middle; }}
:focus-visible {{ outline: 3px solid #000080; outline-offset: 3px; }}
.skip {{ position: absolute; left: .5rem; top: .5rem; padding: .5rem; background: white; transform: translateY(-200%); }}
.skip:focus {{ transform: none; }}
#feedback {{ min-height: 3.2em; }}
</style></head><body><a class="skip" href="#main">Skip to main content</a>
<main id="main" tabindex="-1">{content}</main><script nonce="{nonce}">{script}</script></body></html>"##
    )
}
/// Visible username/password form with native labels, autocomplete and validation.
pub(super) fn welcome(nonce: &str) -> String {
    page(
        "Collect your café order",
        r#"<h1>Collect your café order</h1>
<p>Sign in, check your ready order, then confirm you have received your drinks. Reloading keeps your collection recorded.</p>
<p>Local demo account: username <strong>maya</strong>, password <strong>Cafe-Demo-Pickup-42!</strong>. Maya has two oat lattes on ticket 104. These credentials belong only to this local example.</p>
<form id="login" action="/identity/login" method="post" aria-describedby="login-help feedback">
<p id="login-help">Both fields are required. You can paste the demo password or use your password manager.</p>
<label for="username">Username (required)</label><input id="username" name="username" autocomplete="username" aria-describedby="login-help feedback" required maxlength="128">
<label for="password">Password (required)</label><input id="password" name="password" type="password" autocomplete="current-password" aria-describedby="login-help feedback" required maxlength="1024">
<button id="sign-in" type="submit">Sign in and view my order</button>
</form><p id="feedback" role="status" aria-atomic="true"></p>
<noscript><p>JavaScript is required to send the native antiforgery token with these forms. Enable it and reload to sign in.</p></noscript>
<p><a href="/pickup">View my pickup order</a> · <a href="/menu">Read the café menu</a></p>"#,
        nonce,
    )
}
/// Render a verified customer's order with a deliberate receipt confirmation.
pub(super) fn order(
    ticket: i64,
    drink: &str,
    cups: i64,
    collected: bool,
    announcement: &str,
    nonce: &str,
) -> String {
    let drink = escape(drink);
    let announcement = escape(announcement);
    let (state, action) = if collected {
        (
            "Collected",
            String::from(
                "<p>This order has already been collected. No further collection is needed.</p>",
            ),
        )
    } else {
        (
            "Ready",
            String::from(
                r#"<form id="collect" action="/identity/pickup/collect" method="post" aria-describedby="collect-help feedback">
<p id="collect-help">Confirm only after the barista has handed you your drinks. This records collection and cannot be undone in this example.</p>
<label for="received"><input id="received" name="received" type="checkbox" required> I have received all my drinks (required)</label>
<button id="collect-order" type="submit">Confirm collection</button></form>"#,
            ),
        )
    };
    let content = format!(
        r#"<h1>Your pickup order</h1><p>{announcement}</p>
<dl><dt>Ticket</dt><dd>{ticket}</dd><dt>Drink</dt><dd>{drink}</dd><dt>Cups</dt><dd>{cups}</dd><dt>Status</dt><dd id="order-state">{state}</dd></dl>
{action}<p id="feedback" role="status" aria-atomic="true"></p><p><a href="/pickup">Reload my order</a> · <a href="/menu">Read the café menu</a></p>"#
    );
    page("Your pickup order", &content, nonce)
}
/// Dynamic values appear only in HTML text nodes, never in script or attributes.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A denied or failed browser request retains its useful message and sign-in link.
pub(super) fn failure(message: &str) -> String {
    let message = escape(message);
    format!(
        r##"<!doctype html><html lang="en-US"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Request could not be completed — Neighborhood café</title></head><body><a href="#main">Skip to main content</a><main id="main" tabindex="-1"><h1>Request could not be completed</h1><p id="request-error">{message}</p><p><a href="/">Return to café sign-in</a></p></main></body></html>"##
    )
}
