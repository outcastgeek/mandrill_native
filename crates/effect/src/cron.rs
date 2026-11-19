//! Pure recurring-calendar evaluation and occurrence policies.

use crate::UtcMillis;
use crate::memory::MemoryError;
use crate::stream::{PreparedChunkPool, PreparedChunkValues};
use chrono::{
    DateTime, Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone,
    Timelike, Utc,
};
use std::num::NonZeroUsize;

// Chrono's fixed offsets are strictly within one civil day. External zone
// implementations of CalendarZone promise the same bound. This is a calendar
// domain limit used to include both sides of a local-time overlap, not a scan
// batch size or an implicit scheduling policy.
const SECONDS_PER_DAY: i64 = 24 * 60 * 60;
const MILLIS_PER_SECOND: i128 = 1_000;
const MAX_SECOND: u32 = 59;
const MAX_MINUTE: u32 = 59;
const MAX_HOUR: u32 = 23;
const MIN_DAY: u32 = 1;
const MAX_DAY: u32 = 31;
const MIN_MONTH: u32 = 1;
const MAX_MONTH: u32 = 12;
const MAX_WEEKDAY: u32 = 7;
const SUNDAY_ALIAS: u32 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A calendar or time-zone conversion failure with a typed cause.
pub enum CalendarTimeError {
    /// The local date time failed validation.
    InvalidLocalDateTime,
    /// The offset failed validation.
    InvalidOffset,
    /// The supplied value lies outside the supported range.
    OutOfRange,
}

impl std::fmt::Display for CalendarTimeError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for CalendarTimeError {}

/// A validated Gregorian local second. It is pure calendar data, not a clock
/// reading and not a stable instant until a zone resolves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDateTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl CivilDateTime {
    /// Construct a `CivilDateTime` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `year` (`i32`): The year supplied to this operation.
    /// - `month` (`u32`): The month supplied to this operation.
    /// - `day` (`u32`): The day supplied to this operation.
    /// - `hour` (`u32`): The hour supplied to this operation.
    /// - `minute` (`u32`): The minute supplied to this operation.
    /// - `second` (`u32`): The second supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, CalendarTimeError>`.
    pub fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Result<Self, CalendarTimeError> {
        let local = NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, second))
            .ok_or(CalendarTimeError::InvalidLocalDateTime)?;
        Ok(Self::from_naive(local))
    }

    ///
    /// # Parameters
    /// - `local` (`NaiveDateTime`): Local civil-time representation to resolve.
    ///
    /// # Returns
    /// Type: `Self`.
    fn from_naive(local: NaiveDateTime) -> Self {
        Self {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: local.hour(),
            minute: local.minute(),
            second: local.second(),
        }
    }

    ///
    /// # Returns
    /// Type: `Result<NaiveDateTime, CalendarTimeError>`.
    fn to_naive(self) -> Result<NaiveDateTime, CalendarTimeError> {
        NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .and_then(|date| date.and_hms_opt(self.hour, self.minute, self.second))
            .ok_or(CalendarTimeError::InvalidLocalDateTime)
    }

    #[must_use]
    /// # Returns
    /// Type: `(i32, u32, u32, u32, u32, u32)`.
    pub const fn components(self) -> (i32, u32, u32, u32, u32, u32) {
        (
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The possible UTC resolutions of one local civil time, including gaps and overlaps.
pub enum LocalResolution {
    /// The local time does not exist because the clock moved forward.
    Gap,
    /// The identity was accepted as a new record. Contains `UtcMillis`.
    Unique(UtcMillis),
    /// The local time occurs twice because the clock moved backward.
    Overlap {
        /// Earlier UTC realization of an overlapped local time. Type: `UtcMillis`.
        earlier: UtcMillis,
        /// Later UTC realization of an overlapped local time. Type: `UtcMillis`.
        later: UtcMillis,
    },
}

/// A pure, externally implementable zone resolver. Every resolved offset must
/// be strictly less than one day in magnitude. The same logical zone/version
/// must be supplied during replay; no process-local timezone is consulted.
pub trait CalendarZone: Send + Sync {
    /// Provider-specific failure type returned by this capability.
    type Error;

    /// # Parameters
    /// - `local` (`CivilDateTime`): Local civil-time representation to resolve.
    ///
    /// # Returns
    /// Type: `Result<LocalResolution, Self::Error>`.
    fn resolve(&self, local: CivilDateTime) -> Result<LocalResolution, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A calendar zone with one constant UTC offset.
pub struct FixedOffsetZone(FixedOffset);

impl FixedOffsetZone {
    /// Return the fixed offset in seconds east of UTC.
    ///
    /// # Parameters
    /// - `seconds` (`i32`): The seconds supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, CalendarTimeError>`.
    pub fn east_seconds(seconds: i32) -> Result<Self, CalendarTimeError> {
        FixedOffset::east_opt(seconds)
            .map(Self)
            .ok_or(CalendarTimeError::InvalidOffset)
    }

    #[must_use]
    /// # Returns
    /// Type: `i32`.
    pub const fn seconds_east(self) -> i32 {
        self.0.local_minus_utc()
    }
}

impl CalendarZone for FixedOffsetZone {
    /// Type alias for error in cron.
    type Error = CalendarTimeError;

    ///
    /// # Parameters
    /// - `local` (`CivilDateTime`): Local civil-time representation to resolve.
    ///
    /// # Returns
    /// Type: `Result<LocalResolution, Self::Error>`.
    fn resolve(&self, local: CivilDateTime) -> Result<LocalResolution, Self::Error> {
        let local = local.to_naive()?;
        Ok(match self.0.from_local_datetime(&local) {
            LocalResult::None => return Err(CalendarTimeError::OutOfRange),
            LocalResult::Single(instant) => {
                LocalResolution::Unique(UtcMillis::new(instant.timestamp_millis()))
            }
            LocalResult::Ambiguous(first, second) => {
                let first = UtcMillis::new(first.timestamp_millis());
                let second = UtcMillis::new(second.timestamp_millis());
                LocalResolution::Overlap {
                    earlier: first.min(second),
                    later: first.max(second),
                }
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The rule for combining day-of-month and day-of-week constraints.
pub enum DayMatch {
    /// A match in either configured day field is sufficient.
    Either,
    /// Both compared conditions hold.
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The choice applied when a local time does not exist after a clock transition.
pub enum GapPolicy {
    /// Leave the occurrence or value unchanged and continue.
    Skip,
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The choice applied when a local time occurs twice after a clock transition.
pub enum OverlapPolicy {
    /// Select the earlier UTC realization of an overlapped time.
    Earlier,
    /// Select the later UTC realization of an overlapped time.
    Later,
    /// Both compared conditions hold.
    Both,
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Caller-selected handling of daylight-saving gaps and overlaps.
pub struct CalendarTransitionPolicy {
    /// Behavior for a local time skipped by a clock transition. Type: `GapPolicy`.
    pub gap: GapPolicy,
    /// Behavior for a local time repeated by a clock transition. Type: `OverlapPolicy`.
    pub overlap: OverlapPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The decision for a scheduled occurrence missed while execution was unavailable.
pub enum MissedOccurrencePolicy {
    /// Leave the occurrence or value unchanged and continue.
    Skip,
    /// Select the later realization of an overlapped local time.
    Latest,
    /// Include every matching candidate.
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit resource bounds for cron.
pub struct CronSearchBudget {
    /// Maximum calendar days examined during occurrence search. Type: `NonZeroUsize`.
    pub days: NonZeroUsize,
    /// Maximum candidate instants examined during occurrence search. Type: `NonZeroUsize`.
    pub candidates: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The resource dimension that exhausted a cron search budget.
pub enum CronBudgetKind {
    /// The day-count search budget was exhausted.
    Days,
    /// The candidate-count search budget was exhausted.
    Candidates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The calendar field named by a cron parse or validation issue.
pub enum CronFieldKind {
    /// The second branch or value was selected.
    Second,
    /// The minute field caused this calendar decision.
    Minute,
    /// The hour field caused this calendar decision.
    Hour,
    /// The day field caused this calendar decision.
    Day,
    /// The month field caused this calendar decision.
    Month,
    /// The weekday field caused this calendar decision.
    Weekday,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed reason why a cron expression cannot be parsed.
pub enum CronParseErrorKind {
    /// The calendar expression contains the wrong number of fields.
    FieldCount,
    /// A required calendar field has no value.
    EmptyField,
    /// The number failed validation.
    InvalidNumber,
    /// The range failed validation.
    InvalidRange,
    /// The step failed validation.
    InvalidStep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The position, field, and typed cause of an invalid cron expression.
pub struct CronParseError {
    /// Calendar field associated with the parse failure. Type: `Option<CronFieldKind>`.
    pub field: Option<CronFieldKind>,
    /// Typed discriminator for this state. Type: `CronParseErrorKind`.
    pub kind: CronParseErrorKind,
}

impl std::fmt::Display for CronParseError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for CronParseError {}

#[derive(Debug)]
/// A typed failure while resolving future occurrences under the selected calendar policy.
///
/// # Type parameters
/// - `ZoneError`: Error returned by the calendar-zone provider.
pub enum CronEvalError<ZoneError> {
    /// The requested time window ends before it begins.
    ReversedWindow,
    /// The requested instant lies outside the supported range.
    TimeOutOfRange,
    /// The configured search budget was exhausted. Retains the `CronBudgetKind` detail.
    BudgetExceeded(CronBudgetKind),
    /// The local time falls in a clock-transition gap. Retains the `CivilDateTime` detail.
    Gap(CivilDateTime),
    /// The local time has two valid realizations. Retains the `CivilDateTime` detail.
    Overlap(CivilDateTime),
    /// The zone resolution failed validation.
    InvalidZoneResolution,
    /// The time-zone provider could not resolve the local time. Retains the `ZoneError` detail.
    Zone(ZoneError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The specific calendar or budget condition that stopped occurrence search.
pub enum CronEvalErrorKind {
    /// The end of the requested time window precedes its start.
    ReversedWindow,
    /// The requested instant lies outside the supported range.
    TimeOutOfRange,
    /// The configured search budget was exhausted.
    BudgetExceeded,
    /// The local time does not exist because the clock moved forward.
    Gap,
    /// The local time occurs twice because the clock moved backward.
    Overlap,
    /// The zone resolution failed validation.
    InvalidZoneResolution,
    /// The time-zone provider failed to resolve this occurrence.
    Zone,
}

impl<ZoneError> CronEvalError<ZoneError> {
    #[must_use]
    /// # Type parameters
    /// - `ZoneError`: Error returned by the calendar-zone provider.
    ///
    /// # Returns
    /// Type: `CronEvalErrorKind`.
    pub const fn kind(&self) -> CronEvalErrorKind {
        match self {
            Self::ReversedWindow => CronEvalErrorKind::ReversedWindow,
            Self::TimeOutOfRange => CronEvalErrorKind::TimeOutOfRange,
            Self::BudgetExceeded(_) => CronEvalErrorKind::BudgetExceeded,
            Self::Gap(_) => CronEvalErrorKind::Gap,
            Self::Overlap(_) => CronEvalErrorKind::Overlap,
            Self::InvalidZoneResolution => CronEvalErrorKind::InvalidZoneResolution,
            Self::Zone(_) => CronEvalErrorKind::Zone,
        }
    }
}

impl<ZoneError> std::fmt::Display for CronEvalError<ZoneError> {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.kind(), formatter)
    }
}

impl<ZoneError: std::fmt::Debug> std::error::Error for CronEvalError<ZoneError> {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether an occurrence is the earlier or later realization of an overlapped local time.
pub enum OccurrencePhase {
    /// The identity was accepted as a new record.
    Unique,
    /// Select the earlier UTC realization of an overlapped time.
    Earlier,
    /// Select the later UTC realization of an overlapped time.
    Later,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A resolved occurrence with both its UTC instant and local calendar representation.
pub struct CalendarOccurrence {
    /// Resolved absolute instant. Type: `UtcMillis`.
    pub instant: UtcMillis,
    /// Local civil-time representation of the occurrence. Type: `CivilDateTime`.
    pub local: CivilDateTime,
    /// Current phase of this lifecycle. Type: `OccurrencePhase`.
    pub phase: OccurrencePhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Typed field set used by cron.
struct FieldSet {
    mask: u64,
    wildcard: bool,
}

impl FieldSet {
    ///
    /// # Parameters
    /// - `value` (`u32`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    const fn single(value: u32) -> Self {
        Self {
            mask: 1_u64 << value,
            wildcard: false,
        }
    }

    ///
    /// # Parameters
    /// - `value` (`u32`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    const fn contains(self, value: u32) -> bool {
        self.mask & (1_u64 << value) != 0
    }

    ///
    /// # Parameters
    /// - `raw` (`&str`): The raw supplied to this operation.
    /// - `field` (`CronFieldKind`): The field supplied to this operation.
    /// - `minimum` (`u32`): Lower bound enforced by this policy.
    /// - `maximum` (`u32`): Upper bound enforced by this policy.
    ///
    /// # Returns
    /// Type: `Result<Self, CronParseError>`.
    fn parse(
        raw: &str,
        field: CronFieldKind,
        minimum: u32,
        maximum: u32,
    ) -> Result<Self, CronParseError> {
        let error = |kind| CronParseError {
            field: Some(field),
            kind,
        };
        if raw.is_empty() {
            return Err(error(CronParseErrorKind::EmptyField));
        }
        let mut mask = 0_u64;
        for part in raw.split(',') {
            if part.is_empty() {
                return Err(error(CronParseErrorKind::EmptyField));
            }
            let (base, step) = match part.split_once('/') {
                Some((base, step)) => {
                    let value = step
                        .parse::<u32>()
                        .map_err(|_| error(CronParseErrorKind::InvalidStep))?;
                    if value == 0 || base.is_empty() {
                        return Err(error(CronParseErrorKind::InvalidStep));
                    }
                    (base, Some(value))
                }
                None => (part, None),
            };
            let number = |text: &str| {
                text.parse::<u32>()
                    .map_err(|_| error(CronParseErrorKind::InvalidNumber))
            };
            let (start, end) = if base == "*" {
                (minimum, maximum)
            } else if let Some((start, end)) = base.split_once('-') {
                (number(start)?, number(end)?)
            } else {
                let start = number(base)?;
                (start, if step.is_some() { maximum } else { start })
            };
            if start < minimum || end > maximum || start > end {
                return Err(error(CronParseErrorKind::InvalidRange));
            }
            let step = step.unwrap_or(1);
            let mut value = start;
            loop {
                let bit = if field == CronFieldKind::Weekday && value == SUNDAY_ALIAS {
                    0
                } else {
                    value
                };
                mask |= 1_u64 << bit;
                let Some(next) = value.checked_add(step) else {
                    break;
                };
                if next > end {
                    break;
                }
                value = next;
            }
        }
        let all_values = (minimum..=maximum).fold(0_u64, |allowed, value| {
            let bit = if field == CronFieldKind::Weekday && value == SUNDAY_ALIAS {
                0
            } else {
                value
            };
            allowed | (1_u64 << bit)
        });
        Ok(Self {
            mask,
            wildcard: mask == all_values,
        })
    }
}

/// Six-field cron grammar: second, minute, hour, day, month, weekday. Five
/// fields use the standard zero-second interpretation. Day/weekday conjunction
/// and disjunction are selected explicitly, never inferred from a host locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cron {
    seconds: FieldSet,
    minutes: FieldSet,
    hours: FieldSet,
    days: FieldSet,
    months: FieldSet,
    weekdays: FieldSet,
    day_match: DayMatch,
}

impl Cron {
    /// Parse and validate the input under this schema or syntax policy.
    ///
    /// # Parameters
    /// - `expression` (`&str`): The expression supplied to this operation.
    /// - `day_match` (`DayMatch`): The day match supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, CronParseError>`.
    pub fn parse(expression: &str, day_match: DayMatch) -> Result<Self, CronParseError> {
        let mut fields = expression.split_whitespace();
        let fields: [_; 7] = std::array::from_fn(|_| fields.next());
        let (seconds, [minute, hour, day, month, weekday]) = match fields {
            [
                Some(minute),
                Some(hour),
                Some(day),
                Some(month),
                Some(weekday),
                None,
                None,
            ] => (FieldSet::single(0), [minute, hour, day, month, weekday]),
            [
                Some(second),
                Some(minute),
                Some(hour),
                Some(day),
                Some(month),
                Some(weekday),
                None,
            ] => (
                FieldSet::parse(second, CronFieldKind::Second, 0, MAX_SECOND)?,
                [minute, hour, day, month, weekday],
            ),
            _ => {
                return Err(CronParseError {
                    field: None,
                    kind: CronParseErrorKind::FieldCount,
                });
            }
        };
        Ok(Self {
            seconds,
            minutes: FieldSet::parse(minute, CronFieldKind::Minute, 0, MAX_MINUTE)?,
            hours: FieldSet::parse(hour, CronFieldKind::Hour, 0, MAX_HOUR)?,
            days: FieldSet::parse(day, CronFieldKind::Day, MIN_DAY, MAX_DAY)?,
            months: FieldSet::parse(month, CronFieldKind::Month, MIN_MONTH, MAX_MONTH)?,
            weekdays: FieldSet::parse(weekday, CronFieldKind::Weekday, 0, MAX_WEEKDAY)?,
            day_match,
        })
    }

    ///
    /// # Parameters
    /// - `date` (`NaiveDate`): The date supplied to this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    fn matches_date(&self, date: NaiveDate) -> bool {
        if !self.months.contains(date.month()) {
            return false;
        }
        let day = self.days.contains(date.day());
        let weekday = self
            .weekdays
            .contains(date.weekday().num_days_from_sunday());
        match (self.days.wildcard, self.weekdays.wildcard) {
            (true, true) => true,
            (true, false) => weekday,
            (false, true) => day,
            (false, false) => match self.day_match {
                DayMatch::Either => day || weekday,
                DayMatch::Both => day && weekday,
            },
        }
    }

    /// Enumerate all occurrences after `after` and through `through`, both UTC.
    /// Budgets are caller supplied; exceeding one is a typed incomplete search,
    /// never a silent claim that no occurrence exists. No clock is read here.
    ///
    /// # Type parameters
    /// - `Zone`: Bounds: `CalendarZone`.
    ///
    /// # Parameters
    /// - `after` (`UtcMillis`): Cursor or instant after which results are selected.
    /// - `through` (`UtcMillis`): Last cursor or sequence included in this bounded operation.
    /// - `zone` (`&Zone`): The zone supplied to this operation.
    /// - `transitions` (`CalendarTransitionPolicy`): The transitions supplied to this operation.
    /// - `budget` (`CronSearchBudget`): The budget supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<CalendarOccurrence>, CronEvalError<Zone::Error>>`.
    pub fn occurrences_between<Zone: CalendarZone>(
        &self,
        after: UtcMillis,
        through: UtcMillis,
        zone: &Zone,
        transitions: CalendarTransitionPolicy,
        budget: CronSearchBudget,
    ) -> Result<Vec<CalendarOccurrence>, CronEvalError<Zone::Error>> {
        let mut occurrences = Vec::new();
        self.scan_occurrences(after, through, zone, transitions, budget, |occurrence| {
            occurrences.push(occurrence);
            Ok::<_, CronEvalError<Zone::Error>>(())
        })?;
        sort_occurrences(&mut occurrences)?;
        Ok(occurrences)
    }

    /// Resolve the same UTC-ordered occurrences directly into one regional page.
    /// The page is reserved before consulting the zone provider and remains
    /// checked out while the caller owns it. Exhaustion returns no partial page.
    pub fn occurrences_between_prepared<Zone: CalendarZone>(
        &self,
        after: UtcMillis,
        through: UtcMillis,
        zone: &Zone,
        transitions: CalendarTransitionPolicy,
        budget: CronSearchBudget,
        output: &PreparedChunkPool<CalendarOccurrence>,
    ) -> Result<PreparedChunkValues<CalendarOccurrence>, PreparedCronError<Zone::Error>> {
        if after > through {
            return Err(CronEvalError::ReversedWindow.into());
        }
        let mut occurrences = output.try_checkout().map_err(PreparedCronError::Memory)?;
        self.scan_occurrences(after, through, zone, transitions, budget, |occurrence| {
            occurrences
                .try_push(occurrence)
                .map_err(|_| PreparedCronError::Memory(MemoryError::CollectionReservationFailed))
        })?;
        sort_occurrences(occurrences.as_mut_slice())?;
        Ok(occurrences)
    }

    fn scan_occurrences<Zone: CalendarZone, Error: From<CronEvalError<Zone::Error>>>(
        &self,
        after: UtcMillis,
        through: UtcMillis,
        zone: &Zone,
        transitions: CalendarTransitionPolicy,
        budget: CronSearchBudget,
        mut emit: impl FnMut(CalendarOccurrence) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if after > through {
            return Err(CronEvalError::ReversedWindow.into());
        }
        if after == through {
            return Ok(());
        }
        let lower = DateTime::<Utc>::from_timestamp_millis(after.value())
            .and_then(|instant| {
                instant
                    .naive_utc()
                    .checked_sub_signed(TimeDelta::seconds(SECONDS_PER_DAY))
            })
            .ok_or(CronEvalError::TimeOutOfRange)?;
        let upper = DateTime::<Utc>::from_timestamp_millis(through.value())
            .and_then(|instant| {
                instant
                    .naive_utc()
                    .checked_add_signed(TimeDelta::seconds(SECONDS_PER_DAY))
            })
            .ok_or(CronEvalError::TimeOutOfRange)?;
        let mut date = lower.date();
        let last_date = upper.date();
        let mut days = 0_usize;
        let mut candidates = 0_usize;
        loop {
            days = days
                .checked_add(1)
                .ok_or(CronEvalError::BudgetExceeded(CronBudgetKind::Days))?;
            if days > budget.days.get() {
                return Err(CronEvalError::BudgetExceeded(CronBudgetKind::Days).into());
            }
            if self.matches_date(date) {
                for hour in 0..=MAX_HOUR {
                    if !self.hours.contains(hour) {
                        continue;
                    }
                    for minute in 0..=MAX_MINUTE {
                        if !self.minutes.contains(minute) {
                            continue;
                        }
                        for second in 0..=MAX_SECOND {
                            if !self.seconds.contains(second) {
                                continue;
                            }
                            let local = date
                                .and_hms_opt(hour, minute, second)
                                .ok_or(CronEvalError::TimeOutOfRange)?;
                            if local < lower || local > upper {
                                continue;
                            }
                            candidates = candidates
                                .checked_add(1)
                                .ok_or(CronEvalError::BudgetExceeded(CronBudgetKind::Candidates))?;
                            if candidates > budget.candidates.get() {
                                return Err(CronEvalError::BudgetExceeded(
                                    CronBudgetKind::Candidates,
                                )
                                .into());
                            }
                            let civil = CivilDateTime::from_naive(local);
                            collect_resolution(
                                &mut emit,
                                local,
                                zone.resolve(civil).map_err(CronEvalError::Zone)?,
                                after,
                                through,
                                transitions,
                            )?;
                        }
                    }
                }
            }
            if date == last_date {
                break;
            }
            date = date.succ_opt().ok_or(CronEvalError::TimeOutOfRange)?;
        }
        Ok(())
    }

    /// Select the explicit missed-run policy after a completed occurrence.
    /// `Skip` returns only an occurrence due exactly at `now`, without
    /// backfill; `Latest` reports the most recent due occurrence, while `All`
    /// reports every due occurrence within the budget.
    ///
    /// # Type parameters
    /// - `Zone`: Bounds: `CalendarZone`.
    ///
    /// # Parameters
    /// - `after` (`UtcMillis`): Cursor or instant after which results are selected.
    /// - `now` (`UtcMillis`): The caller-supplied current time.
    /// - `zone` (`&Zone`): The zone supplied to this operation.
    /// - `transitions` (`CalendarTransitionPolicy`): The transitions supplied to this operation.
    /// - `missed` (`MissedOccurrencePolicy`): The missed supplied to this operation.
    /// - `budget` (`CronSearchBudget`): The budget supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<CalendarOccurrence>, CronEvalError<Zone::Error>>`.
    pub fn due<Zone: CalendarZone>(
        &self,
        after: UtcMillis,
        now: UtcMillis,
        zone: &Zone,
        transitions: CalendarTransitionPolicy,
        missed: MissedOccurrencePolicy,
        budget: CronSearchBudget,
    ) -> Result<Vec<CalendarOccurrence>, CronEvalError<Zone::Error>> {
        let mut due = self.occurrences_between(after, now, zone, transitions, budget)?;
        match missed {
            MissedOccurrencePolicy::Skip => {
                due.retain(|occurrence| occurrence.instant == now);
                Ok(due)
            }
            MissedOccurrencePolicy::Latest => Ok(due.pop().into_iter().collect()),
            MissedOccurrencePolicy::All => Ok(due),
        }
    }

    /// Select missed occurrences from the same prepared page used for duplicate
    /// resolution checks and ordering. The page must fit the complete search
    /// result even when the selected policy returns only its latest occurrence.
    pub fn due_prepared<Zone: CalendarZone>(
        &self,
        window: std::ops::RangeInclusive<UtcMillis>,
        zone: &Zone,
        transitions: CalendarTransitionPolicy,
        missed: MissedOccurrencePolicy,
        budget: CronSearchBudget,
        output: &PreparedChunkPool<CalendarOccurrence>,
    ) -> Result<PreparedChunkValues<CalendarOccurrence>, PreparedCronError<Zone::Error>> {
        let (after, now) = window.into_inner();
        let mut due =
            self.occurrences_between_prepared(after, now, zone, transitions, budget, output)?;
        match missed {
            MissedOccurrencePolicy::Skip | MissedOccurrencePolicy::Latest => {
                let latest =
                    due.as_slice().last().copied().filter(|last| {
                        missed == MissedOccurrencePolicy::Latest || last.instant == now
                    });
                drop(due.take_items());
                if let Some(latest) = latest {
                    due.push_reserved(latest);
                }
            }
            MissedOccurrencePolicy::All => {}
        }
        Ok(due)
    }
}

/// Failure of the prepared occurrence search or its selected storage.
#[derive(Debug)]
pub enum PreparedCronError<ZoneError> {
    /// Existing calendar validation or provider failure.
    Calendar(CronEvalError<ZoneError>),
    /// The caller-selected regional page could not hold the search result.
    Memory(MemoryError),
}

impl<ZoneError> From<CronEvalError<ZoneError>> for PreparedCronError<ZoneError> {
    fn from(error: CronEvalError<ZoneError>) -> Self {
        Self::Calendar(error)
    }
}

impl<ZoneError> std::fmt::Display for PreparedCronError<ZoneError> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Calendar(error) => error.fmt(formatter),
            Self::Memory(error) => error.fmt(formatter),
        }
    }
}

impl<ZoneError: std::fmt::Debug> std::error::Error for PreparedCronError<ZoneError> {}

fn sort_occurrences<ZoneError>(
    occurrences: &mut [CalendarOccurrence],
) -> Result<(), CronEvalError<ZoneError>> {
    occurrences.sort_unstable_by_key(|occurrence| occurrence.instant);
    if occurrences.windows(2).any(|pair| match pair {
        [first, second] => first.instant == second.instant,
        _ => false,
    }) {
        return Err(CronEvalError::InvalidZoneResolution);
    }
    Ok(())
}

///
/// # Parameters
/// - `instant` (`UtcMillis`): The instant supplied to this operation.
/// - `after` (`UtcMillis`): Cursor or instant after which results are selected.
/// - `through` (`UtcMillis`): Last cursor or sequence included in this bounded operation.
///
/// # Returns
/// Type: `bool`.
/// Returns whether the requested condition holds.
fn in_window(instant: UtcMillis, after: UtcMillis, through: UtcMillis) -> bool {
    instant > after && instant <= through
}

///
/// # Type parameters
/// - `ZoneError`: Error returned by the calendar-zone provider.
///
/// # Parameters
/// - `local` (`NaiveDateTime`): Local civil-time representation to resolve.
/// - `instant` (`UtcMillis`): The instant supplied to this operation.
///
/// # Returns
/// Type: `Result<(), CronEvalError<ZoneError>>`.
fn validate_resolution<ZoneError>(
    local: NaiveDateTime,
    instant: UtcMillis,
) -> Result<(), CronEvalError<ZoneError>> {
    let local_as_utc = DateTime::<Utc>::from_naive_utc_and_offset(local, Utc);
    let offset_millis = i128::from(local_as_utc.timestamp_millis()) - i128::from(instant.value());
    if offset_millis.abs() >= i128::from(SECONDS_PER_DAY) * MILLIS_PER_SECOND
        || offset_millis % MILLIS_PER_SECOND != 0
    {
        return Err(CronEvalError::InvalidZoneResolution);
    }
    Ok(())
}

///
/// # Type parameters
/// - `ZoneError`: Error returned by the calendar-zone provider.
///
/// # Parameters
/// - `emit`: Destination for occurrences that fall in the requested window.
/// - `local` (`NaiveDateTime`): Local civil-time representation to resolve.
/// - `resolution` (`LocalResolution`): The resolution supplied to this operation.
/// - `after` (`UtcMillis`): Cursor or instant after which results are selected.
/// - `through` (`UtcMillis`): Last cursor or sequence included in this bounded operation.
/// - `transitions` (`CalendarTransitionPolicy`): The transitions supplied to this operation.
///
/// # Returns
/// Type: `Result<(), Error>`.
fn collect_resolution<ZoneError, Error: From<CronEvalError<ZoneError>>>(
    emit: &mut impl FnMut(CalendarOccurrence) -> Result<(), Error>,
    local: NaiveDateTime,
    resolution: LocalResolution,
    after: UtcMillis,
    through: UtcMillis,
    transitions: CalendarTransitionPolicy,
) -> Result<(), Error> {
    let civil = CivilDateTime::from_naive(local);
    let selected = match resolution {
        LocalResolution::Gap => {
            return if transitions.gap == GapPolicy::Reject {
                Err(CronEvalError::Gap(civil).into())
            } else {
                Ok(())
            };
        }
        LocalResolution::Unique(instant) => {
            validate_resolution(local, instant)?;
            [Some((instant, OccurrencePhase::Unique)), None]
        }
        LocalResolution::Overlap { earlier, later } => {
            if earlier >= later {
                return Err(CronEvalError::InvalidZoneResolution.into());
            }
            validate_resolution(local, earlier)?;
            validate_resolution(local, later)?;
            match transitions.overlap {
                OverlapPolicy::Earlier => [Some((earlier, OccurrencePhase::Earlier)), None],
                OverlapPolicy::Later => [Some((later, OccurrencePhase::Later)), None],
                OverlapPolicy::Both => [
                    Some((earlier, OccurrencePhase::Earlier)),
                    Some((later, OccurrencePhase::Later)),
                ],
                OverlapPolicy::Reject => {
                    return if in_window(earlier, after, through) || in_window(later, after, through)
                    {
                        Err(CronEvalError::Overlap(civil).into())
                    } else {
                        Ok(())
                    };
                }
            }
        }
    };
    for (instant, phase) in selected.into_iter().flatten() {
        if in_window(instant, after, through) {
            emit(CalendarOccurrence {
                instant,
                local: civil,
                phase,
            })?;
        }
    }
    Ok(())
}
