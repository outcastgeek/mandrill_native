//! Percent-style formatting for explicitly supplied string operands.

use std::fmt;

#[expect(
    clippy::expect_used,
    reason = "the constant printable-character expression is reviewed"
)]
fn quote(output: &mut dyn fmt::Write, value: &str, ascii: bool, raw: bool) -> fmt::Result {
    if raw
        && value
            .chars()
            .all(|c| c != '`' && c != '\u{feff}' && (c == '\t' || (c >= ' ' && c != '\u{7f}')))
    {
        return write!(output, "`{value}`");
    }
    static PRINTABLE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S} ]$").expect("printable Unicode categories")
    });
    output.write_char('"')?;
    for c in value.chars() {
        match c {
            '"' => output.write_str("\\\"")?,
            '\\' => output.write_str("\\\\")?,
            '\u{7}' => output.write_str("\\a")?,
            '\u{8}' => output.write_str("\\b")?,
            '\u{c}' => output.write_str("\\f")?,
            '\n' => output.write_str("\\n")?,
            '\r' => output.write_str("\\r")?,
            '\t' => output.write_str("\\t")?,
            '\u{b}' => output.write_str("\\v")?,
            _ => {
                let mut encoded = [0; 4];
                if (!ascii || c.is_ascii()) && PRINTABLE.is_match(c.encode_utf8(&mut encoded)) {
                    output.write_char(c)?;
                } else if (c as u32) < 0x20 || c == '\u{7f}' {
                    write!(output, "\\x{:02x}", c as u32)?;
                } else if (c as u32) <= 0xffff {
                    write!(output, "\\u{:04x}", c as u32)?;
                } else {
                    write!(output, "\\U{:08x}", c as u32)?;
                }
            }
        }
    }
    output.write_char('"')
}

#[derive(Default)]
struct CharCount(usize);
impl fmt::Write for CharCount {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 = self.0.checked_add(text.chars().count()).ok_or(fmt::Error)?;
        Ok(())
    }
}

fn padding(output: &mut dyn fmt::Write, mut count: usize, zero: bool) -> fmt::Result {
    let block = if zero {
        "00000000000000000000000000000000"
    } else {
        "                                "
    };
    while count >= block.len() {
        output.write_str(block)?;
        count -= block.len();
    }
    output.write_str(block.get(..count).ok_or(fmt::Error)?)
}

fn write_padded(
    output: &mut dyn fmt::Write,
    width: Option<usize>,
    left: bool,
    zero: bool,
    render: impl Fn(&mut dyn fmt::Write) -> fmt::Result,
) -> fmt::Result {
    let Some(width) = width else {
        return render(output);
    };
    let mut size = CharCount::default();
    render(&mut size)?;
    let count = width.saturating_sub(size.0);
    if left {
        render(output)?;
        padding(output, count, false)
    } else {
        padding(output, count, zero)?;
        render(output)
    }
}

fn next(input: &mut &str) -> Option<char> {
    let character = input.chars().next()?;
    *input = input.get(character.len_utf8()..)?;
    Some(character)
}

fn strip(input: &mut &str, prefix: char) -> bool {
    if let Some(rest) = input.strip_prefix(prefix) {
        *input = rest;
        true
    } else {
        false
    }
}

fn number(input: &mut &str) -> Option<usize> {
    let initial = *input;
    let mut value = 0usize;
    while let Some(digit) = input.as_bytes().first().copied().filter(u8::is_ascii_digit) {
        if value > 1_000_000 {
            *input = "";
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(usize::from(digit - b'0'));
        let _ = next(input);
    }
    (initial.len() != input.len()).then_some(value)
}

struct Operand {
    next: usize,
    reordered: bool,
}

fn index(input: &mut &str, operand: &mut Operand, valid: &mut bool, count: usize) -> bool {
    if !strip(input, '[') {
        return false;
    }
    operand.reordered = true;
    if input.chars().take(2).count() < 2 {
        *valid = false;
        return false;
    }
    let Some((mut digits, tail)) = input.split_once(']') else {
        *valid = false;
        return false;
    };
    let value = number(&mut digits);
    let parsed = value.is_some() && digits.is_empty();
    if let Some(value) = value.filter(|value| parsed && *value > 0 && *value <= count) {
        operand.next = value - 1;
    } else {
        *valid = false;
    }
    *input = tail;
    parsed
}

fn precision_prefix(value: &str, precision: Option<usize>) -> &str {
    let Some(limit) = precision else { return value };
    let end = value
        .char_indices()
        .nth(limit)
        .map_or(value.len(), |(start, _)| start);
    value.get(..end).unwrap_or(value)
}

#[derive(Default)]
struct Flags {
    left: bool,
    zero: bool,
    sharp: bool,
    plus: bool,
    space: bool,
}
impl Flags {
    fn read(input: &mut &str) -> Self {
        let mut flags = Self::default();
        while let Some(character) = input.chars().next() {
            match character {
                '-' => flags.left = true,
                '0' => flags.zero = true,
                '#' => flags.sharp = true,
                '+' => flags.plus = true,
                ' ' => flags.space = true,
                _ => break,
            }
            let _ = next(input);
        }
        flags
    }
}

pub(crate) fn format_one_string(label: &str, language: &str) -> String {
    format_strings(label, &[language])
}

pub(crate) fn format_strings(label: &str, arguments: &[&str]) -> String {
    let mut output = String::new();
    let _ = write_format_strings(label, arguments, &mut output);
    output
}

/// Format directly into the selected destination, without a temporary quoted,
/// padded, or hexadecimal operand. A finite writer can reject any expansion.
pub(crate) fn write_format_strings(
    mut input: &str,
    arguments: &[&str],
    output: &mut dyn fmt::Write,
) -> fmt::Result {
    let mut operand = Operand {
        next: 0,
        reordered: false,
    };
    while let Some(character) = next(&mut input) {
        if character != '%' {
            output.write_char(character)?;
            continue;
        }
        let flags = Flags::read(&mut input);
        let mut valid = true;
        let mut after_index = index(&mut input, &mut operand, &mut valid, arguments.len());
        let width = if strip(&mut input, '*') {
            operand.next = operand.next.saturating_add(1);
            after_index = false;
            output.write_str("%!(BADWIDTH)")?;
            None
        } else {
            let width = number(&mut input);
            if after_index && width.is_some() {
                valid = false;
            }
            width
        };
        let precision =
            if input.get(1..).is_some_and(|tail| !tail.is_empty()) && strip(&mut input, '.') {
                if after_index {
                    valid = false;
                }
                after_index = index(&mut input, &mut operand, &mut valid, arguments.len());
                if strip(&mut input, '*') {
                    operand.next = operand.next.saturating_add(1);
                    after_index = false;
                    output.write_str("%!(BADPREC)")?;
                    None
                } else {
                    Some(number(&mut input).unwrap_or_default())
                }
            } else {
                None
            };
        if !after_index {
            index(&mut input, &mut operand, &mut valid, arguments.len());
        }
        let Some(verb) = next(&mut input) else {
            output.write_str("%!(NOVERB)")?;
            break;
        };
        if verb == '%' {
            output.write_char('%')?;
            continue;
        }
        if !valid {
            write!(output, "%!{verb}(BADINDEX)")?;
            continue;
        }
        let Some(&argument) = arguments.get(operand.next) else {
            write!(output, "%!{verb}(MISSING)")?;
            continue;
        };
        operand.next += 1;
        let render = |output: &mut dyn fmt::Write| {
            let text = precision_prefix(argument, precision);
            match verb {
                's' => output.write_str(text),
                'v' if !flags.sharp => output.write_str(text),
                'v' => quote(output, text, false, false),
                'q' => quote(output, text, flags.plus, flags.sharp),
                'T' => output.write_str(precision_prefix("string", precision)),
                'x' | 'X' => write_hex(output, argument, precision, &flags, verb == 'x'),
                'w' if flags.sharp => quote(output, text, false, false),
                _ => output.write_str(text),
            }
        };
        let unknown = !matches!(verb, 's' | 'v' | 'q' | 'T' | 'x' | 'X');
        if unknown {
            write!(output, "%!{verb}(string=")?;
        }
        write_padded(output, width, flags.left, flags.zero, render)?;
        if unknown {
            output.write_char(')')?;
        }
    }
    if let Some(extra) = arguments.get(operand.next..)
        && !operand.reordered
        && !extra.is_empty()
    {
        output.write_str("%!(EXTRA ")?;
        for (index, value) in extra.iter().enumerate() {
            if index > 0 {
                output.write_str(", ")?;
            }
            write!(output, "string={value}")?;
        }
        output.write_char(')')?;
    }
    Ok(())
}

fn write_hex(
    output: &mut dyn fmt::Write,
    argument: &str,
    limit: Option<usize>,
    flags: &Flags,
    lowercase: bool,
) -> fmt::Result {
    let prefix = if lowercase { "0x" } else { "0X" };
    for (index, byte) in argument
        .bytes()
        .take(limit.unwrap_or(usize::MAX))
        .enumerate()
    {
        if flags.space && index > 0 {
            output.write_char(' ')?;
        }
        if flags.sharp && (flags.space || index == 0) {
            output.write_str(prefix)?;
        }
        if lowercase {
            write!(output, "{byte:02x}")?;
        } else {
            write!(output, "{byte:02X}")?;
        }
    }
    Ok(())
}
