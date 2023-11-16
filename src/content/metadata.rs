use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::ProcessContent;

#[derive(Debug)]
pub(crate) struct DirectoryMetadata {
    pub subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
    pub files: Arc<OnceLock<Vec<FileMetadata>>>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FileMetadata {
    pub draft: bool,
    pub slug: String,
    pub path: Utf8PathBuf,
    pub title: Option<String>,
    pub date: Option<DateTime<Utc>>,

    // further data from frontmatter that should be printed in dump-metadata
    // but not passed to the template as `page.*`
    #[serde(skip)]
    pub template: Option<Utf8PathBuf>,
    #[serde(skip)]
    pub process_content: Option<ProcessContent>,
    #[serde(skip)]
    pub syntax_highlight_theme: Option<String>,
}

pub(super) fn metadata_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::empty();

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

    // Own filters
    env.add_filter("date_prefix", date_prefix);
    env.add_filter("strip_date_prefix", strip_date_prefix);

    env
}

fn date_prefix(value: String) -> minijinja::Value {
    match split_date_prefix(&value) {
        Some((date, _rest)) => date.into(),
        None => minijinja::Value::UNDEFINED,
    }
}

fn strip_date_prefix(value: String) -> String {
    match split_date_prefix(&value) {
        Some((_date, rest)) => rest.to_owned(),
        None => value,
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
        assert_eq!(strip_date_prefix("1111-11-11-11".to_owned()), "11".to_owned());
        assert_eq!(strip_date_prefix("1-1-1-test".to_owned()), "test".to_owned());
        assert_eq!(strip_date_prefix("2023-01-01".to_owned()), "2023-01-01".to_owned());
    }
}
