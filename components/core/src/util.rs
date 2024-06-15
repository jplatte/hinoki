use std::{fmt, str::FromStr};

use anyhow::anyhow;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use toml::value::{Date, Offset, Time};

#[derive(Debug)]
pub(crate) struct OrderBiMap {
    pub ordered_to_original: Vec<usize>,
    pub original_to_ordered: Vec<usize>,
}

impl OrderBiMap {
    pub(crate) fn new<T, K: Ord>(original: &[T], key_fn: impl Fn(&T) -> K) -> Self {
        let ordered_to_original: Vec<_> = original
            .iter()
            .enumerate()
            .sorted_by_key(|(_, item)| key_fn(item))
            .map(|(idx, _)| idx)
            .collect();

        let mut original_to_ordered = vec![0; original.len()];
        for (ordered, &original) in ordered_to_original.iter().enumerate() {
            original_to_ordered[original] = ordered;
        }

        Self { ordered_to_original, original_to_ordered }
    }
}

/// Like toml::value::Datetime, but with the date being required.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HinokiDatetime {
    pub date: Date,
    pub time: Option<Time>,
    pub offset: Option<Offset>,
}

impl From<HinokiDatetime> for toml::value::Datetime {
    fn from(val: HinokiDatetime) -> Self {
        Self { date: Some(val.date), time: val.time, offset: val.offset }
    }
}

impl fmt::Debug for HinokiDatetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        toml::value::Datetime::from(*self).fmt(f)
    }
}

impl FromStr for HinokiDatetime {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let dt: toml::value::Datetime = s.parse()?;
        let date = dt.date.ok_or_else(|| anyhow!("missing date"))?;
        Ok(Self { date, time: dt.time, offset: dt.offset })
    }
}

impl<'de> Deserialize<'de> for HinokiDatetime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Deserialize is only used in TOML context, as part of FileContentDatetime.
        // The potential of a string input is handled in that type, the native toml
        // datetime is handled here.
        let dt = toml::value::Datetime::deserialize(deserializer)?;
        let date = dt.date.ok_or_else(|| serde::de::Error::custom("missing date"))?;
        Ok(Self { date, time: dt.time, offset: dt.offset })
    }
}

impl Serialize for HinokiDatetime {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize is only used in minijinja context, where the toml datetime
        // serialization format is `{"$__toml_private_datetime": "..."}`, which
        // is not very helpful. Stringify and serialize the string instead.
        let datetime = toml::value::Datetime::from(*self);
        datetime.to_string().serialize(serializer)
    }
}
