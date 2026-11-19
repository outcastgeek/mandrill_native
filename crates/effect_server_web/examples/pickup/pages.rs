//! Small semantic HTML views; dynamic text is escaped before entering trusted markup.

use effect_server::ServerError;
use effect_server_web::rendering::PreparedHtmlWriter;

/// One accessible shell; static script/styles use the native per-response CSP nonce.
fn page<E: From<ServerError>>(
    out: &mut PreparedHtmlWriter<'_>,
    title: &str,
    nonce: &str,
    content: impl FnOnce(&mut PreparedHtmlWriter<'_>) -> Result<(), E>,
) -> Result<(), E> {
    out.write_trusted(r##"<!doctype html>
<html lang="en-US"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>"##)?;
    out.write_text(title)?;
    out.write_trusted(
        r##" — Neighborhood café</title>
<style nonce=""##,
    )?;
    out.write_text(nonce)?;
    out.write_trusted(r##"">
:root { color-scheme: light; }
body { margin: 0; background: white; color: black; font: 1rem/1.6 system-ui, sans-serif; }
main { max-width: 38rem; margin: auto; padding: 1.5rem; overflow-wrap: anywhere; }
a { color: #000080; }
a:visited { color: #000080; }
label { display: block; margin-top: 1rem; }
input,button { font: inherit; color: black; background: white; border: 2px solid black; border-radius: .25rem; padding: .65rem; box-sizing: border-box; min-height: 44px; }
input { display: block; width: 100%; }
button { margin-top: 1rem; cursor: pointer; }
button[aria-disabled=true] { color: black; border-style: dashed; cursor: default; }
input[type=checkbox] { display: inline-block; width: 44px; height: 44px; vertical-align: middle; }
:focus-visible { outline: 3px solid #000080; outline-offset: 3px; }
.skip { position: absolute; left: .5rem; top: .5rem; padding: .5rem; background: white; transform: translateY(-200%); }
.skip:focus { transform: none; }
#feedback { min-height: 3.2em; }
</style></head><body><a class="skip" href="#main">Skip to main content</a>
<main id="main" tabindex="-1">"##)?;
    content(out)?;
    out.write_trusted(r##"</main><script nonce=""##)?;
    out.write_text(nonce)?;
    out.write_trusted(r##"">"##)?;
    out.write_trusted(include_str!("browser.js"))?;
    Ok(out.write_trusted(r##"</script></body></html>"##)?)
}
/// Visible username/password form with native labels, autocomplete and validation.
pub(super) fn welcome(out: &mut PreparedHtmlWriter<'_>, nonce: &str) -> Result<(), ServerError> {
    page(out, "Collect your café order", nonce, |out| {
        out.write_trusted(r#"<h1>Collect your café order</h1>
<p>Sign in, check your ready order, then confirm you have received your drinks. Reloading keeps your collection recorded.</p>
<p>Local demo account: username <strong>maya</strong>, password <strong>Cafe-Demo-Pickup-42!</strong>. Maya has two oat lattes on ticket 104. These credentials belong only to this local example.</p>
<form id="login" action="/identity/login" method="post" aria-describedby="login-help feedback">
<p id="login-help">Both fields are required. You can paste the demo password or use your password manager.</p>
<label for="username">Username (required)</label><input id="username" name="username" autocomplete="username" aria-describedby="login-help feedback" required maxlength="128">
<label for="password">Password (required)</label><input id="password" name="password" type="password" autocomplete="current-password" aria-describedby="login-help feedback" required maxlength="1024">
<button id="sign-in" type="submit">Sign in and view my order</button>
</form><p id="feedback" role="status" aria-atomic="true"></p>
<noscript><p>JavaScript is required to send the native antiforgery token with these forms. Enable it and reload to sign in.</p></noscript>
<p><a href="/pickup">View my pickup order</a> · <a href="/menu">Read the café menu</a></p>"#)
    })
}
/// Render a verified customer's order with a deliberate receipt confirmation.
pub(super) fn order<E: From<ServerError>>(
    out: &mut PreparedHtmlWriter<'_>,
    (ticket, drink, cups, collected): (i64, &str, i64, bool),
    nonce: &str,
    announcement: impl FnOnce(&mut PreparedHtmlWriter<'_>) -> Result<(), E>,
) -> Result<(), E> {
    let (state, action) = if collected {
        (
            "Collected",
            "<p>This order has already been collected. No further collection is needed.</p>",
        )
    } else {
        (
            "Ready",
            r#"<form id="collect" action="/identity/pickup/collect" method="post" aria-describedby="collect-help feedback">
<p id="collect-help">Confirm only after the barista has handed you your drinks. This records collection and cannot be undone in this example.</p>
<label for="received"><input id="received" name="received" type="checkbox" required> I have received all my drinks (required)</label>
<button id="collect-order" type="submit">Confirm collection</button></form>"#,
        )
    };
    page(out, "Your pickup order", nonce, |out| {
        out.write_trusted("<h1>Your pickup order</h1><p>")?;
        announcement(out)?;
        out.write_trusted(
            "</p>
<dl><dt>Ticket</dt><dd>",
        )?;
        out.write_text_fmt(format_args!("{ticket}"))?;
        out.write_trusted("</dd><dt>Drink</dt><dd>")?;
        out.write_text(drink)?;
        out.write_trusted("</dd><dt>Cups</dt><dd>")?;
        out.write_text_fmt(format_args!("{cups}"))?;
        out.write_trusted(r#"</dd><dt>Status</dt><dd id="order-state">"#)?;
        out.write_text(state)?;
        out.write_trusted(
            "</dd></dl>
",
        )?;
        out.write_trusted(action)?;
        Ok(out.write_trusted(r#"<p id="feedback" role="status" aria-atomic="true"></p><p><a href="/pickup">Reload my order</a> · <a href="/menu">Read the café menu</a></p>"#)?)
    })
}
/// Dynamic values appear only in HTML text nodes, never in script or attributes.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A denied or failed browser request retains its useful message and sign-in link.
macro_rules! failure_open {
    () => { r##"<!doctype html><html lang="en-US"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Request could not be completed — Neighborhood café</title></head><body><a href="#main">Skip to main content</a><main id="main" tabindex="-1"><h1>Request could not be completed</h1><p id="request-error">"## };
}
macro_rules! failure_close {
    () => {
        r#"</p><p><a href="/">Return to café sign-in</a></p></main></body></html>"#
    };
}

const FAILURE_OPEN: &str = failure_open!();
const FAILURE_CLOSE: &str = failure_close!();

#[derive(Clone, Copy)]
pub(crate) struct ErrorPage {
    pub(crate) message: &'static str,
    pub(crate) html: &'static str,
}

macro_rules! stock_error {
    ($message:literal) => {
        ErrorPage {
            message: $message,
            html: concat!(failure_open!(), $message, failure_close!()),
        }
    };
}

impl ErrorPage {
    pub(crate) const SIGN_IN: Self = stock_error!("Sign in to view your own pickup order.");
    pub(crate) const SECURITY: Self =
        stock_error!("The secure request was rejected. Reload the page and try again.");
    pub(crate) const LOGIN_FIELDS: Self =
        stock_error!("Invalid login fields. Enter a username and password.");
    pub(crate) const LOGIN_SIZE: Self =
        stock_error!("The login request is too large. Shorten the fields and try again.");
    pub(crate) const NO_ORDER: Self = stock_error!("No pickup order belongs to this account.");
    pub(crate) const UNKNOWN: Self =
        stock_error!("The café could not complete this request. Try again.");
}

pub(super) fn failure(message: &str) -> String {
    let message = escape(message);
    format!("{FAILURE_OPEN}{message}{FAILURE_CLOSE}")
}
