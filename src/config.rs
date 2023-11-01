use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use indexmap::{indexmap, IndexMap};
use serde::{de, Deserialize, Deserializer};

use crate::content::ProcessContent;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default = "default_output_dir")]
    pub output_dir: Utf8PathBuf,
    #[serde(default)]
    pub defaults: Defaults,
}

fn default_output_dir() -> Utf8PathBuf {
    "build".into()
}

pub(crate) struct Defaults {
    values: Vec<DefaultsForPath>,
    globset: GlobSet,
}

impl Defaults {
    pub(crate) fn from_map(map: IndexMap<String, DefaultsForPath>) -> Result<Self, globset::Error> {
        let mut builder = GlobSetBuilder::new();
        for path_glob in map.keys() {
            builder.add(Glob::new(path_glob)?);
        }
        let globset = builder.build()?;
        let values = map.into_values().collect();
        Ok(Self { values, globset })
    }

    pub(crate) fn for_path(
        &self,
        path: &Utf8Path,
    ) -> impl Iterator<Item = &DefaultsForPath> + DoubleEndedIterator {
        self.globset.matches(path).into_iter().map(|idx| &self.values[idx])
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self::from_map(indexmap! {
            "*.md".to_owned() => DefaultsForPath {
                process_content: Some(ProcessContent::MarkdownToHtml),
                ..Default::default()
            }
        })
        .unwrap()
    }
}

impl<'de> Deserialize<'de> for Defaults {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: IndexMap<String, DefaultsForPath> = IndexMap::deserialize(deserializer)?;
        Self::from_map(map).map_err(de::Error::custom)
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultsForPath {
    pub path: Option<Utf8PathBuf>,
    pub template: Option<Utf8PathBuf>,
    pub process_content: Option<ProcessContent>,
    pub title: Option<String>,
    pub date: Option<DateTime<Utc>>,
    pub slug: Option<String>,
}
