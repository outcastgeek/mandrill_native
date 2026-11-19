use std::cell::RefCell;

use maud::{Markup, PreEscaped, Render};

/// Tracks the closing quotes stock Maud writes for the generated attribute slots.
#[derive(Default)]
pub struct AttributeSlots {
    quotes: RefCell<Vec<usize>>,
}

impl AttributeSlots {
    #[must_use]
    pub fn spread<I>(&self, attributes: I) -> Spread<'_, I> {
        Spread {
            slots: self,
            attributes: RefCell::new(Some(attributes)),
        }
    }

    #[must_use]
    pub fn finish(self, markup: Markup) -> Markup {
        let mut quotes = self.quotes.into_inner().into_iter().peekable();
        let mut output = markup.into_string();
        if quotes.peek().is_none() {
            return PreEscaped(output);
        }
        let mut offset = 0;
        output.retain(|character| {
            let keep = quotes.peek() != Some(&offset);
            if !keep {
                debug_assert_eq!(character, '"');
                let _ = quotes.next();
            }
            offset += character.len_utf8();
            keep
        });
        PreEscaped(output)
    }
}

/// Maud renders this in place, between the surrounding named attributes.
pub struct Spread<'a, I> {
    slots: &'a AttributeSlots,
    attributes: RefCell<Option<I>>,
}

impl<I, N, V> Render for Spread<'_, I>
where
    I: IntoIterator<Item = (N, V)>,
    N: AsRef<str>,
    V: AsRef<str>,
{
    fn render_to(&self, output: &mut String) {
        let Some(attributes) = self.attributes.borrow_mut().take() else {
            return;
        };
        let prefix = " data-web-components-spread=\"";
        assert!(output.ends_with(prefix), "unexpected Maud attribute slot");
        output.truncate(output.len() - prefix.len());
        for (name, value) in attributes {
            let name = name.as_ref();
            let value = value.as_ref();
            assert!(
                !name.is_empty()
                    && !name.chars().any(|character| {
                        character.is_whitespace()
                            || character.is_control()
                            || matches!(character, '\'' | '"' | '<' | '>' | '/' | '=')
                    }),
                "invalid HTML attribute name in Maud attribute spread"
            );
            output.push(' ');
            name.render_to(output);
            output.push_str("=\"");
            value.render_to(output);
            output.push('"');
        }
        self.slots.quotes.borrow_mut().push(output.len());
    }
}
