use crate::UtcMillis;
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
pub enum CalendarTimeError {
    InvalidLocalDateTime,
    InvalidOffset,
    OutOfRange,
}

impl std::fmt::Display for CalendarTimeError {
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

    fn to_naive(self) -> Result<NaiveDateTime, CalendarTimeError> {
        NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .and_then(|date| date.and_hms_opt(self.hour, self.minute, self.second))
            .ok_or(CalendarTimeError::InvalidLocalDateTime)
    }

    #[must_use]
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
pub enum LocalResolution {
    Gap,
    Unique(UtcMillis),
    Overlap {
        earlier: UtcMillis,
        later: UtcMillis,
    },
}

/// A pure, externally implementable zone resolver. Every resolved offset must
/// be strictly less than one day in magnitude. The same logical zone/version
/// must be supplied during replay; no process-local timezone is consulted.
pub trait CalendarZone: Send + Sync {
    type Error;

    fn resolve(&self, local: CivilDateTime) -> Result<LocalResolution, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedOffsetZone(FixedOffset);

impl FixedOffsetZone {
    pub fn east_seconds(seconds: i32) -> Result<Self, CalendarTimeError> {
        FixedOffset::east_opt(seconds)
            .map(Self)
            .ok_or(CalendarTimeError::InvalidOffset)
    }

    #[must_use]
    pub const fn seconds_east(self) -> i32 {
        self.0.local_minus_utc()
    }
}

impl CalendarZone for FixedOffsetZone {
    type Error = CalendarTimeError;

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
pub enum DayMatch {
    Either,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapPolicy {
    Skip,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlapPolicy {
    Earlier,
    Later,
    Both,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarTransitionPolicy {
    pub gap: GapPolicy,
    pub overlap: OverlapPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissedOccurrencePolicy {
    Skip,
    Latest,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CronSearchBudget {
    pub days: NonZeroUsize,
    pub candidates: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CronBudgetKind {
    Days,
    Candidates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CronFieldKind {
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Weekday,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CronParseErrorKind {
    FieldCount,
    EmptyField,
    InvalidNumber,
    InvalidRange,
    InvalidStep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CronParseError {
    pub field: Option<CronFieldKind>,
    pub kind: CronParseErrorKind,
}

impl std::fmt::Display for CronParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for CronParseError {}

#[derive(Debug)]
pub enum CronEvalError<ZoneError> {
    ReversedWindow,
    TimeOutOfRange,
    BudgetExceeded(CronBudgetKind),
    Gap(CivilDateTime),
    Overlap(CivilDateTime),
    InvalidZoneResolution,
    Zone(ZoneError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CronEvalErrorKind {
    ReversedWindow,
    TimeOutOfRange,
    BudgetExceeded,
    Gap,
    Overlap,
    InvalidZoneResolution,
    Zone,
}

impl<ZoneError> CronEvalError<ZoneError> {
    #[must_use]
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
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.kind(), formatter)
    }
}

impl<ZoneError: std::fmt::Debug> std::error::Error for CronEvalError<ZoneError> {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccurrencePhase {
    Unique,
    Earlier,
    Later,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarOccurrence {
    pub instant: UtcMillis,
    pub local: CivilDateTime,
    pub phase: OccurrencePhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldSet {
    mask: u64,
    wildcard: bool,
}

impl FieldSet {
    const fn single(value: u32) -> Self {
        Self {
            mask: 1_u64 << value,
            wildcard: false,
        }
    }

    const fn contains(self, value: u32) -> bool {
        self.mask & (1_u64 << value) != 0
    }

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
    pub fn parse(expression: &str, day_match: DayMatch) -> Result<Self, CronParseError> {
        let fields: Vec<_> = expression.split_whitespace().collect();
        let (seconds, [minute, hour, day, month, weekday]) = match fields.as_slice() {
            [minute, hour, day, month, weekday] => (
                FieldSet::single(0),
                [*minute, *hour, *day, *month, *weekday],
            ),
            [second, minute, hour, day, month, weekday] => (
                FieldSet::parse(second, CronFieldKind::Second, 0, MAX_SECOND)?,
                [*minute, *hour, *day, *month, *weekday],
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
    pub fn occurrences_between<Zone: CalendarZone>(
        &self,
        after: UtcMillis,
        through: UtcMillis,
        zone: &Zone,
        transitions: CalendarTransitionPolicy,
        budget: CronSearchBudget,
    ) -> Result<Vec<CalendarOccurrence>, CronEvalError<Zone::Error>> {
        if after > through {
            return Err(CronEvalError::ReversedWindow);
        }
        if after == through {
            return Ok(Vec::new());
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
        let mut occurrences = Vec::new();
        loop {
            days = days
                .checked_add(1)
                .ok_or(CronEvalError::BudgetExceeded(CronBudgetKind::Days))?;
            if days > budget.days.get() {
                return Err(CronEvalError::BudgetExceeded(CronBudgetKind::Days));
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
                                ));
                            }
                            let civil = CivilDateTime::from_naive(local);
                            collect_resolution(
                                &mut occurrences,
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
        occurrences.sort_unstable_by_key(|occurrence| occurrence.instant);
        if occurrences.windows(2).any(|pair| match pair {
            [first, second] => first.instant == second.instant,
            _ => false,
        }) {
            return Err(CronEvalError::InvalidZoneResolution);
        }
        Ok(occurrences)
    }

    /// Select the explicit missed-run policy after a completed occurrence.
    /// `Skip` returns only an occurrence due exactly at `now`, without
    /// backfill; `Latest` reports the most recent due occurrence, while `All`
    /// reports every due occurrence within the budget.
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
}

fn in_window(instant: UtcMillis, after: UtcMillis, through: UtcMillis) -> bool {
    instant > after && instant <= through
}

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

fn collect_resolution<ZoneError>(
    occurrences: &mut Vec<CalendarOccurrence>,
    local: NaiveDateTime,
    resolution: LocalResolution,
    after: UtcMillis,
    through: UtcMillis,
    transitions: CalendarTransitionPolicy,
) -> Result<(), CronEvalError<ZoneError>> {
    let civil = CivilDateTime::from_naive(local);
    let selected = match resolution {
        LocalResolution::Gap => {
            return if transitions.gap == GapPolicy::Reject {
                Err(CronEvalError::Gap(civil))
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
                return Err(CronEvalError::InvalidZoneResolution);
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
                        Err(CronEvalError::Overlap(civil))
                    } else {
                        Ok(())
                    };
                }
            }
        }
    };
    for (instant, phase) in selected.into_iter().flatten() {
        push_occurrence(occurrences, after, through, instant, civil, phase);
    }
    Ok(())
}

fn push_occurrence(
    occurrences: &mut Vec<CalendarOccurrence>,
    after: UtcMillis,
    through: UtcMillis,
    instant: UtcMillis,
    local: CivilDateTime,
    phase: OccurrencePhase,
) {
    if in_window(instant, after, through) {
        occurrences.push(CalendarOccurrence {
            instant,
            local,
            phase,
        });
    }
}
