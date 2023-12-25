use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use camino::Utf8PathBuf;
use indexmap::IndexMap;
use minijinja::UndefinedBehavior;
use serde::Serialize;
use time::Date;

pub(super) fn metadata_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::empty();

    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_loader(|tpl| Ok(Some(tpl.to_owned())));
    env.set_syntax(minijinja::Syntax {
        block_start: "{%".into(),
        block_end: "%}".into(),
        variable_start: "{".into(),
        variable_end: "}".into(),
        comment_start: "{#".into(),
        comment_end: "#}".into(),
    })
    .expect("custom minijinja syntax is valid");

    env.add_filter("default", minijinja::filters::default);
    env.add_filter("first", minijinja::filters::first);
    env.add_filter("join", minijinja::filters::join);
    env.add_filter("last", minijinja::filters::last);
    env.add_filter("replace", minijinja::filters::replace);
    env.add_filter("reverse", minijinja::filters::reverse);
    env.add_filter("sort", minijinja::filters::sort);
    env.add_filter("trim", minijinja::filters::trim);

    #[cfg(feature = "datetime")]
    {
        use minijinja_contrib::filters as contrib_filters;

        env.add_filter("dateformat", contrib_filters::dateformat);
        env.add_filter("datetimeformat", contrib_filters::datetimeformat);
        env.add_filter("timeformat", contrib_filters::timeformat);
    }

    // Own filters
    env.add_filter("date_prefix", date_prefix);
    env.add_filter("strip_date_prefix", strip_date_prefix);

    env
}

fn date_prefix(value: Cow<'_, str>) -> minijinja::Value {
    match split_date_prefix(&value) {
        Some((date, _rest)) => date.into(),
        None => minijinja::Value::UNDEFINED,
    }
}

fn strip_date_prefix(value: Cow<'_, str>) -> String {
    match split_date_prefix(&value) {
        Some((_date, rest)) => rest.to_owned(),
        None => value.into_owned(),
    }
}

fn split_date_prefix(value: &str) -> Option<(&str, &str)> {
    let mut idx = 0;
    let mut digit_seen = false;
    let mut dashes_seen = 0;

    while idx < value.len() {
        let byte = value.as_bytes()[idx];

        if byte.is_ascii_digit() {
            digit_seen = true;
        } else if byte == b'-' && digit_seen {
            dashes_seen += 1;
            if dashes_seen == 3 {
                return Some((&value[..idx], &value[idx + 1..]));
            }
            digit_seen = false;
        } else {
            break;
        }

        idx += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::strip_date_prefix;

    #[test]
    fn strip_date_prefixes() {
        assert_eq!(strip_date_prefix("1111-11-11-11".into()), "11");
        assert_eq!(strip_date_prefix("1-1-1-test".into()), "test");
        assert_eq!(strip_date_prefix("2023-01-01".into()), "2023-01-01");
    }
}
