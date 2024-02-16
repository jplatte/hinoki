use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use indexmap::{indexmap, IndexMap};
use serde::{de, Deserialize, Deserializer};

use crate::content::{ContentFileConfig, ProcessContent};

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_output_dir")]
    pub output_dir: Utf8PathBuf,
    #[serde(default, rename = "content")]
    pub content_file_settings: ContentFileSettings,
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,

    #[serde(skip, default)]
    pub path: Utf8PathBuf,
}

fn default_output_dir() -> Utf8PathBuf {
    "build".into()
}

#[derive(Clone)]
pub struct ContentFileSettings {
    values: Vec<ContentFileConfig>,
    globset: GlobSet,
}

impl ContentFileSettings {
    pub(crate) fn from_map(
        map: IndexMap<String, ContentFileConfig>,
    ) -> Result<Self, globset::Error> {
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
    ) -> impl Iterator<Item = &ContentFileConfig> + DoubleEndedIterator {
        self.globset.matches(path).into_iter().map(|idx| &self.values[idx])
    }
}

impl Default for ContentFileSettings {
    fn default() -> Self {
        Self::from_map(indexmap! {
            "*.md".to_owned() => ContentFileConfig {
                process: Some(ProcessContent::MarkdownToHtml),
                ..Default::default()
            }
        })
        .unwrap()
    }
}

impl<'de> Deserialize<'de> for ContentFileSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: IndexMap<String, ContentFileConfig> = IndexMap::deserialize(deserializer)?;
        Self::from_map(map).map_err(de::Error::custom)
    }
}
