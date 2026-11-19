//! Text form snapshot with sorted offset records and decoded UTF-8 in one admitted slot.
use super::{
    FormPolicy, ServerError, TextUnit, visit_decoded_component, visit_url_encoded_component_pairs,
};
use effect::memory::{BufferLease, BufferView, Bytes, MemoryError};

const WORD: usize = std::mem::size_of::<usize>();
const RECORD: usize = WORD * 4;

pub(super) struct PreparedForm {
    bytes: BufferView<'static>,
    records: usize,
}

pub(super) fn storage_bytes(policy: FormPolicy) -> Result<Bytes, MemoryError> {
    let body = policy.body.bytes();
    // Non-empty segments need at least one encoded byte each and a
    // separating ampersand. Decoding never expands the encoded bytes.
    Bytes::new(body.div_ceil(2))
        .checked_mul(RECORD)?
        .checked_add(Bytes::new(body))
}

impl PreparedForm {
    pub(super) fn parse(
        encoded: &[u8],
        mut output: BufferLease<'static>,
        policy: FormPolicy,
        reject_raw_semicolons: bool,
    ) -> Result<Self, ServerError> {
        std::str::from_utf8(encoded).map_err(|_| invalid_utf8())?;
        output.clear();
        let pairs = encoded
            .split(|byte| *byte == b'&')
            .filter(|part| !part.is_empty())
            .count();
        let records = pairs
            .checked_mul(RECORD)
            .ok_or(ServerError::Memory(MemoryError::ArithmeticOverflow))?;
        for _ in 0..pairs {
            output
                .extend_from_slice(&[0; RECORD])
                .map_err(ServerError::Memory)?;
        }
        let mut offset = 0;
        visit_url_encoded_component_pairs(encoded, reject_raw_semicolons, |key, value| {
            let start = output.len() - records;
            append_component(&mut output, key)?;
            let middle = output.len() - records;
            append_component(&mut output, value)?;
            let end = output.len() - records;
            let index = offset / RECORD;
            let destination = output
                .as_mut_slice()
                .get_mut(offset..offset + RECORD)
                .ok_or(ServerError::Memory(MemoryError::InvalidRange))?;
            let (words, _) = destination.as_chunks_mut::<WORD>();
            for (word, number) in words.iter_mut().zip([start, middle, end, index]) {
                word.copy_from_slice(&number.to_ne_bytes());
            }
            offset += RECORD;
            Ok(())
        })?;

        let (table, values) = output.as_mut_slice().split_at_mut(records);
        let (table, _) = table.as_chunks_mut::<RECORD>();
        table.sort_unstable_by(|left, right| {
            field(left, values, false)
                .cmp(&field(right, values, false))
                .then_with(|| word(left, 3).cmp(&word(right, 3)))
        });
        let form = Self {
            bytes: BufferView::new(output.share()),
            records,
        };
        form.check(policy)?;
        Ok(form)
    }

    pub(super) fn try_clone(&self) -> Result<Self, ServerError> {
        Ok(Self {
            bytes: self.bytes.try_clone().map_err(ServerError::Memory)?,
            records: self.records,
        })
    }

    pub(super) fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        let (table, values) = self.bytes.as_ref().split_at(self.records);
        let (table, _) = table.as_chunks::<RECORD>();
        // Parsing validates and initializes every range before freezing this view.
        table.iter().filter_map(move |record| {
            Some((field(record, values, false)?, field(record, values, true)?))
        })
    }

    fn check(&self, policy: FormPolicy) -> Result<(), ServerError> {
        let mut distinct = 0;
        let mut previous = None;
        for (name, _) in self.pairs() {
            if previous != Some(name) {
                distinct += 1;
                previous = Some(name);
            }
        }
        if distinct > policy.fields.get() {
            return Err(ServerError::Limit("form fields".into()));
        }
        let mut pairs = self.pairs().peekable();
        while let Some((name, value)) = pairs.next() {
            let overlong = |value: &str| {
                let length = match policy.values.unit {
                    TextUnit::Bytes => value.len(),
                    TextUnit::Scalars => value.chars().count(),
                };
                length > policy.values.length.get()
            };
            let mut count = 1;
            let mut invalid_length = overlong(value);
            while let Some((_, value)) = pairs.next_if(|(next, _)| *next == name) {
                count += 1;
                invalid_length |= overlong(value);
            }
            if count > policy.values.count.get() {
                return Err(ServerError::Limit(format!("{name} count")));
            }
            if invalid_length {
                return Err(ServerError::Limit(format!("{name} length")));
            }
        }
        Ok(())
    }
}

fn invalid_utf8() -> ServerError {
    ServerError::Invalid("form UTF-8".into())
}

fn append_component(output: &mut BufferLease<'static>, raw: &[u8]) -> Result<(), ServerError> {
    let start = output.len();
    visit_decoded_component(raw, |byte| {
        output
            .extend_from_slice(&[byte])
            .map_err(ServerError::Memory)
    })?;
    let (_, decoded) = output.as_slice().split_at(start);
    std::str::from_utf8(decoded).map_err(|_| invalid_utf8())?;
    Ok(())
}

fn word(record: &[u8; RECORD], index: usize) -> Option<usize> {
    let (words, _) = record.as_chunks::<WORD>();
    words.get(index).copied().map(usize::from_ne_bytes)
}

fn field<'a>(record: &[u8; RECORD], values: &'a [u8], value: bool) -> Option<&'a str> {
    let start = usize::from(value);
    let bytes = values.get(word(record, start)?..word(record, start + 1)?)?;
    std::str::from_utf8(bytes).ok()
}
