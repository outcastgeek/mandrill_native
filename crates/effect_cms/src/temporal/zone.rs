//! IANA time-zone resolution and civil-time projection.
//!
//! Every accepted zone resolves to either an IANA entry or a caller-supplied fixed
//! offset; formatting and parsing use the resolved location, and output is labeled
//! with the zone abbreviation in effect at that instant (`MST`). This module owns
//! that primitive: a resolved [`TemporalZone`], the local civil-time projection
//! [`ZonedDateTime`] it produces, and the wall-clock-to-instant rule.
//!
//! # Wall-clock normalization
//!
//! A wall clock inside a DST gap is not rejected and a DST overlap does not pick
//! earliest/latest. The rule applies the zone offset in effect at the civil fields
//! read as an instant, then re-reads the offset at the *instant that offset implies*.
//! Both stored-value normalization and display parsing use that path, so
//! [`TemporalZone::wall_clock_instant`] implements exactly those two lookups; no
//! reject/earliest/latest policy is invented here.
//!
//! # No hidden clock or zone
//!
//! Nothing in this module reads the system clock and nothing resolves the process
//! zone implicitly: the UNIX-timestamp formatters take the local zone as an explicit
//! argument and the current-time status reads the clock through the caller's
//! environment.

use std::str::FromStr;

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, NaiveDateTime, Offset, TimeZone, Timelike, Utc,
};
use chrono_tz::{OffsetName, Tz};

/// A resolved time zone: an IANA zone or a caller-supplied fixed offset.
///
/// The IANA variant keeps the database's own abbreviation and DST rules. The fixed
/// variant exists for a caller-supplied fixed offset that is not loaded from the
/// database. Building either variant is the caller's explicit choice; this owner
/// applies no fallback zone of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalZone {
    /// A zone resolved from the IANA time-zone database.
    Iana(Tz),
    /// A fixed offset with its zone label.
    Fixed {
        /// Zone label reported when formatting `MST`.
        name: String,
        /// Offset from UTC.
        offset: FixedOffset,
    },
}

impl TemporalZone {
    /// The UTC zone, the database's own `UTC` entry (abbreviation `UTC`).
    #[must_use]
    pub const fn utc() -> Self {
        Self::Iana(Tz::UTC)
    }

    /// Resolve an IANA zone name against the IANA time-zone database.
    ///
    /// Lookup is case-sensitive and validated against the database, so an unknown name
    /// is `None` rather than a silently accepted label.
    #[must_use]
    pub fn iana(raw: &str) -> Option<Self> {
        Tz::from_str(raw).ok().map(Self::Iana)
    }

    /// Build a fixed-offset zone with an explicit label.
    #[must_use]
    pub fn fixed(name: impl Into<String>, offset: FixedOffset) -> Self {
        Self::Fixed {
            name: name.into(),
            offset,
        }
    }

    /// Normalize a time zone.
    ///
    /// Blank or whitespace-only input resolves to `Ok(None)` without a database
    /// lookup; a nonblank name that the database cannot resolve is `Err(())`, which
    /// reported as `invalid time zone` (and classified as an invalid time
    /// zone) before any date/time parsing happens.
    pub(crate) fn normalize(raw: &str) -> Result<Option<Self>, ()> {
        let value = raw.trim();
        if value.is_empty() {
            return Ok(None);
        }
        Self::iana(value).map(Some).ok_or(())
    }

    /// The canonical name this zone reports.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Iana(tz) => tz.name(),
            Self::Fixed { name, .. } => name.as_str(),
        }
    }

    /// Zone abbreviation in effect at a UTC civil instant.
    fn abbreviation_at(&self, utc: NaiveDateTime) -> String {
        match self {
            Self::Iana(tz) => tz
                .offset_from_utc_datetime(&utc)
                .abbreviation()
                .unwrap_or_default()
                .to_owned(),
            Self::Fixed { name, offset } => {
                let label = name.trim();
                if label.is_empty() {
                    fixed_zone_label(offset.local_minus_utc())
                } else {
                    name.clone()
                }
            }
        }
    }

    /// Offset from UTC in effect at a UTC civil instant, in seconds.
    fn offset_seconds_at(&self, utc: NaiveDateTime) -> i32 {
        match self {
            Self::Iana(tz) => tz.offset_from_utc_datetime(&utc).fix().local_minus_utc(),
            Self::Fixed { offset, .. } => offset.local_minus_utc(),
        }
    }

    /// Offset at a UNIX instant, extending the native table's unbounded end spans.
    /// chrono-tz has no POSIX future-rule evaluator; its final table offset remains
    /// in effect beyond the civil range, just as it does within that range.
    pub(crate) fn unix_offset_seconds(&self, seconds: i64) -> i32 {
        let utc = DateTime::from_timestamp(seconds, 0)
            .map(|time| time.naive_utc())
            .unwrap_or(if seconds < 0 {
                NaiveDateTime::MIN
            } else {
                NaiveDateTime::MAX
            });
        self.offset_seconds_at(utc)
    }

    /// Interpret civil wall-clock fields in this zone, with normalization.
    ///
    /// The civil fields are first read as an instant, that instant's zone offset is
    /// applied, and the offset in effect at the resulting instant is used for the
    /// final conversion — the two-lookup rule. A wall clock inside a
    /// DST gap therefore lands on the offset the database resolves for it, instead of
    /// being rejected or clamped by a local policy.
    #[must_use]
    pub(crate) fn wall_clock_instant(&self, wall: NaiveDateTime) -> DateTime<Utc> {
        let first = self.offset_seconds_at(wall);
        let candidate = wall
            .checked_sub_signed(Duration::seconds(i64::from(first)))
            .unwrap_or(wall);
        let resolved = self.offset_seconds_at(candidate);
        let instant = wall
            .checked_sub_signed(Duration::seconds(i64::from(resolved)))
            .unwrap_or(wall);
        DateTime::from_naive_utc_and_offset(instant, Utc)
    }

    /// Project an instant into this zone's civil fields, abbreviation included.
    #[must_use]
    pub(crate) fn local_at(&self, instant: DateTime<Utc>) -> ZonedDateTime {
        let utc = instant.naive_utc();
        let offset_seconds = self.offset_seconds_at(utc);
        let local = utc
            .checked_add_signed(Duration::seconds(i64::from(offset_seconds)))
            .unwrap_or(utc);
        ZonedDateTime {
            local,
            offset_seconds,
            zone_name: self.abbreviation_at(utc),
            zone: self.clone(),
        }
    }
}

/// Render a fixed offset as a nameless fixed zone.
fn fixed_zone_label(offset_seconds: i32) -> String {
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let magnitude = offset_seconds.unsigned_abs();
    let hours = magnitude / 3_600;
    let minutes = (magnitude % 3_600) / 60;
    format!("{sign}{hours:02}{minutes:02}")
}

/// An instant together with the zone it is projected into.
///
/// An instant plus the location its civil fields and abbreviation are read from.
/// passes such a value as `now` to `TemporalStatusAt`; callers here build it from the
/// clock value they selected and the zone they selected.
#[derive(Clone, Debug)]
pub struct TemporalInstant {
    /// Absolute instant.
    instant: DateTime<Utc>,
    /// Zone the civil fields are read from.
    zone: TemporalZone,
}

impl TemporalInstant {
    /// Pair an explicit instant with an explicit zone.
    #[must_use]
    pub fn new(instant: DateTime<Utc>, zone: TemporalZone) -> Self {
        Self { instant, zone }
    }

    /// Build the value from Unix epoch milliseconds, the clock capability's unit.
    ///
    /// A millisecond value outside the representable civil-date range yields `None`
    /// rather than a substituted instant.
    #[must_use]
    pub fn from_utc_millis(millis: i64, zone: TemporalZone) -> Option<Self> {
        let instant = match Utc.timestamp_millis_opt(millis) {
            chrono::LocalResult::Single(instant) => instant,
            chrono::LocalResult::Ambiguous(earliest, _latest) => earliest,
            chrono::LocalResult::None => return None,
        };
        Some(Self { instant, zone })
    }

    /// The absolute instant.
    #[must_use]
    pub fn instant(&self) -> DateTime<Utc> {
        self.instant
    }

    /// The selected zone.
    #[must_use]
    pub fn zone(&self) -> &TemporalZone {
        &self.zone
    }

    /// Civil fields and abbreviation of this instant in its selected zone.
    #[must_use]
    pub fn local(&self) -> ZonedDateTime {
        self.zone.local_at(self.instant)
    }

    /// Project the same instant into another zone, keeping the instant unchanged.
    #[must_use]
    pub fn in_zone(&self, zone: &TemporalZone) -> Self {
        Self {
            instant: self.instant,
            zone: zone.clone(),
        }
    }
}

/// Civil date/time fields of one instant in one zone, with its offset and label.
///
/// Every accessor is read from the projected civil value, so a wall clock that the
/// zone database normalized across a DST boundary reports the normalized fields, as
/// the year/hour reads do.
#[derive(Clone, Debug)]
pub struct ZonedDateTime {
    /// Civil fields in the projected zone.
    local: NaiveDateTime,
    /// Offset from UTC in effect, in seconds.
    offset_seconds: i32,
    /// Zone abbreviation in effect (`MST` label).
    zone_name: String,
    /// Zone the civil fields were projected into.
    zone: TemporalZone,
}

impl ZonedDateTime {
    /// The absolute instant this civil value denotes.
    #[must_use]
    pub fn instant(&self) -> DateTime<Utc> {
        let utc = self
            .local
            .checked_sub_signed(Duration::seconds(i64::from(self.offset_seconds)))
            .unwrap_or(self.local);
        DateTime::from_naive_utc_and_offset(utc, Utc)
    }

    /// The zone this value was projected into.
    #[must_use]
    pub fn zone(&self) -> &TemporalZone {
        &self.zone
    }

    /// Zone abbreviation in effect.
    #[must_use]
    pub fn zone_name(&self) -> &str {
        &self.zone_name
    }

    /// Offset from UTC in effect, in seconds.
    #[must_use]
    pub fn offset_seconds(&self) -> i32 {
        self.offset_seconds
    }

    /// Civil year.
    #[must_use]
    pub fn year(&self) -> i32 {
        self.local.year()
    }

    /// Civil month, `1`–`12`.
    #[must_use]
    pub fn month(&self) -> u32 {
        self.local.month()
    }

    /// Civil day of month.
    #[must_use]
    pub fn day(&self) -> u32 {
        self.local.day()
    }

    /// Civil hour, `0`–`23`.
    #[must_use]
    pub fn hour(&self) -> u32 {
        self.local.hour()
    }

    /// Civil minute.
    #[must_use]
    pub fn minute(&self) -> u32 {
        self.local.minute()
    }

    /// Civil second.
    #[must_use]
    pub fn second(&self) -> u32 {
        self.local.second()
    }

    /// Nanosecond within the civil second.
    #[must_use]
    pub fn nanosecond(&self) -> u32 {
        self.local.and_utc().timestamp_subsec_nanos()
    }

    /// Day of week, `0` = Sunday.
    #[must_use]
    pub fn weekday_number(&self) -> u32 {
        self.local.weekday().num_days_from_sunday()
    }

    /// Day of year, `1`–`366`.
    #[must_use]
    pub fn year_day(&self) -> u32 {
        self.local.ordinal()
    }

    /// Midnight at the start of this civil date in this same zone.
    ///
    /// It is built from the civil date at midnight in the zone, so the same
    /// wall-clock normalization applies.
    #[must_use]
    pub(crate) fn start_of_day(&self) -> DateTime<Utc> {
        let midnight = self.local.date().and_hms_opt(0, 0, 0).unwrap_or(self.local);
        self.zone.wall_clock_instant(midnight)
    }
}
