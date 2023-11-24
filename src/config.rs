use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use indexmap::{indexmap, IndexMap};
use serde::{de, Deserialize, Deserializer};

use crate::content::{FileConfig, ProcessContent};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_output_dir")]
    pub output_dir: Utf8PathBuf,
    #[serde(default, rename = "files")]
    pub file_config_defaults: Files,
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,
}

fn default_output_dir() -> Utf8PathBuf {
    "build".into()
}

pub struct Files {
    values: Vec<FileConfig>,
    globset: GlobSet,
}

impl Files {
    pub(crate) fn from_map(map: IndexMap<String, FileConfig>) -> Result<Self, globset::Error> {
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
    ) -> impl Iterator<Item = &FileConfig> + DoubleEndedIterator {
        self.globset.matches(path).into_iter().map(|idx| &self.values[idx])
    }
}

impl Default for Files {
    fn default() -> Self {
        Self::from_map(indexmap! {
            "*.md".to_owned() => FileConfig {
                process_content: Some(ProcessContent::MarkdownToHtml),
                ..Default::default()
            }
        })
        .unwrap()
    }
}

impl<'de> Deserialize<'de> for Files {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: IndexMap<String, FileConfig> = IndexMap::deserialize(deserializer)?;
        Self::from_map(map).map_err(de::Error::custom)
    }
}
