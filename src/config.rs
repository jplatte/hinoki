use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use indexmap::{indexmap, IndexMap};
use serde::{de, Deserialize, Deserializer};

use crate::content::{Frontmatter, ProcessContent};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_output_dir")]
    pub output_dir: Utf8PathBuf,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,
}

fn default_output_dir() -> Utf8PathBuf {
    "build".into()
}

pub struct Defaults {
    values: Vec<Frontmatter>,
    globset: GlobSet,
}

impl Defaults {
    pub(crate) fn from_map(map: IndexMap<String, Frontmatter>) -> Result<Self, globset::Error> {
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
    ) -> impl Iterator<Item = &Frontmatter> + DoubleEndedIterator {
        self.globset.matches(path).into_iter().map(|idx| &self.values[idx])
    }
}

impl Default for Defaults {
    fn default() -> Self {
        Self::from_map(indexmap! {
            "*.md".to_owned() => Frontmatter {
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
        let map: IndexMap<String, Frontmatter> = IndexMap::deserialize(deserializer)?;
        Self::from_map(map).map_err(de::Error::custom)
    }
}
