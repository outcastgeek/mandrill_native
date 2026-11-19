//! Adapt attribute spreads before delegating the template to Maud.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream as Tokens, TokenTree};
use quote::{ToTokens, quote};
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::{Expr, Pat, Token};

/// Render a Maud template with `..(attributes)` in an opening tag.
/// Each spread evaluates once and emits escaped name/value pairs in iteration order.
/// Stock Maud expands the template, so the calling crate needs a `maud` dependency.
#[proc_macro]
pub fn html(input: TokenStream) -> TokenStream {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: Tokens) -> syn::Result<Tokens> {
    let name = match crate_name("web_components") {
        Ok(FoundCrate::Itself) => "web_components".to_owned(),
        Ok(FoundCrate::Name(name)) => name,
        Err(error) => return Err(syn::Error::new(Span::call_site(), error)),
    };
    let facade = Ident::new(&name, Span::call_site());
    let slots = Ident::new("__web_component_attribute_slots", Span::mixed_site());
    let mut template = SpreadSyntax {
        slots: &slots,
        changed: false,
    };
    let markup = if candidate(input.clone()) {
        (|stream: ParseStream<'_>| template.markup(stream)).parse2(input)?
    } else {
        input
    };
    if template.changed {
        Ok(quote!({
            let #slots = ::#facade::__private::AttributeSlots::default();
            let markup = ::#facade::__private::maud_html! { #markup };
            #slots.finish(markup)
        }))
    } else {
        Ok(quote!(::#facade::__private::maud_html! { #markup }))
    }
}

/// Plain Maud templates can pass through without examining their grammar.
fn candidate(input: Tokens) -> bool {
    let mut dots = 0;
    for token in input {
        if let TokenTree::Group(group) = &token {
            if dots >= 2 && group.delimiter() == Delimiter::Parenthesis {
                return true;
            }
            if group.delimiter() == Delimiter::Brace && candidate(group.stream()) {
                return true;
            }
        }
        dots = if matches!(token, TokenTree::Punct(ref punct) if punct.as_char() == '.') {
            dots + 1
        } else {
            0
        };
    }
    false
}

struct SpreadSyntax<'a> {
    slots: &'a Ident,
    changed: bool,
}

impl SpreadSyntax<'_> {
    fn markup(&mut self, input: ParseStream<'_>) -> syn::Result<Tokens> {
        let mut output = Tokens::new();
        while !input.is_empty() {
            self.element(input, &mut output)?;
        }
        Ok(output)
    }

    fn element(&mut self, input: ParseStream<'_>, output: &mut Tokens) -> syn::Result<()> {
        if input.peek(Token![@]) {
            return self.control(input, output);
        }
        match input.fork().parse::<TokenTree>()? {
            TokenTree::Ident(_) => self.tag(input, output),
            TokenTree::Punct(punct) if matches!(punct.as_char(), '.' | '#') => {
                self.tag(input, output)
            }
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
                self.body(input, output, false)
            }
            TokenTree::Group(_) | TokenTree::Punct(_) | TokenTree::Literal(_) => {
                copy(input, output)
            }
        }
    }

    /// Preserve stock tag syntax. Only a spread in the opening tag is rewritten.
    fn tag(&mut self, input: ParseStream<'_>, output: &mut Tokens) -> syn::Result<()> {
        if spread_ahead(input) {
            // Like the class and ID shorthand, an initial spread uses an implicit div.
            output.extend(quote!(div));
        } else {
            copy(input, output)?;
        }
        let mut attribute_value = false;
        while !input.is_empty() {
            if input.peek(Token![;]) {
                return copy(input, output);
            }
            if input.peek(syn::token::Brace) {
                self.body(input, output, false)?;
                if !attribute_value {
                    return Ok(());
                }
                attribute_value = false;
                continue;
            }
            if spread_ahead(input) {
                input.parse::<Token![..]>()?;
                let expression: Group = input.parse()?;
                let slots = self.slots;
                output.extend(quote!(data-web-components-spread=(#slots.spread #expression)));
                self.changed = true;
                attribute_value = false;
                continue;
            }
            let token: TokenTree = input.parse()?;
            attribute_value = matches!(&token, TokenTree::Punct(punct) if punct.as_char() == '=');
            output.extend([token]);
        }
        Ok(())
    }

    /// Parse only the Rust boundaries of control forms. Rust ranges and splices
    /// inside those expressions stay opaque to the attribute pass.
    fn control(&mut self, input: ParseStream<'_>, output: &mut Tokens) -> syn::Result<()> {
        input.parse::<Token![@]>()?.to_tokens(output);
        let keyword = input.call(Ident::parse_any)?;
        keyword.to_tokens(output);
        match keyword.to_string().as_str() {
            "if" | "while" => Self::condition(input, output)?,
            "for" => {
                input
                    .call(Pat::parse_multi_with_leading_vert)?
                    .to_tokens(output);
                input.parse::<Token![in]>()?.to_tokens(output);
                Self::condition(input, output)?;
            }
            "else" => {
                if input.peek(Token![if]) {
                    input.parse::<Token![if]>()?.to_tokens(output);
                    Self::condition(input, output)?;
                }
            }
            "match" => {
                Self::condition(input, output)?;
                return self.body(input, output, true);
            }
            "let" => {
                while !input.is_empty() && !input.peek(Token![;]) {
                    copy(input, output)?;
                }
                return copy(input, output);
            }
            _ => return Ok(()),
        }
        self.body(input, output, false)
    }

    fn condition(input: ParseStream<'_>, output: &mut Tokens) -> syn::Result<()> {
        input
            .call(Expr::parse_without_eager_brace)?
            .to_tokens(output);
        Ok(())
    }

    fn body(
        &mut self,
        input: ParseStream<'_>,
        output: &mut Tokens,
        match_arms: bool,
    ) -> syn::Result<()> {
        let group: Group = input.parse()?;
        if group.delimiter() != Delimiter::Brace {
            return Err(syn::Error::new(group.span(), "expected a Maud body"));
        }
        let body = (|stream: ParseStream<'_>| {
            if match_arms {
                self.arms(stream)
            } else {
                self.markup(stream)
            }
        })
        .parse2(group.stream())?;
        let mut body_group = Group::new(Delimiter::Brace, body);
        body_group.set_span(group.span());
        output.extend([TokenTree::Group(body_group)]);
        Ok(())
    }

    fn arms(&mut self, input: ParseStream<'_>) -> syn::Result<Tokens> {
        let mut output = Tokens::new();
        while !input.is_empty() {
            input
                .call(Pat::parse_multi_with_leading_vert)?
                .to_tokens(&mut output);
            if input.peek(Token![if]) {
                input.parse::<Token![if]>()?.to_tokens(&mut output);
                Self::condition(input, &mut output)?;
            }
            input.parse::<Token![=>]>()?.to_tokens(&mut output);
            self.element(input, &mut output)?;
            if input.peek(Token![,]) {
                copy(input, &mut output)?;
            }
        }
        Ok(output)
    }
}

fn spread_ahead(input: ParseStream<'_>) -> bool {
    let probe = input.fork();
    probe.parse::<Token![..]>().is_ok() && probe.peek(syn::token::Paren)
}

fn copy(input: ParseStream<'_>, output: &mut Tokens) -> syn::Result<()> {
    output.extend([input.parse::<TokenTree>()?]);
    Ok(())
}
