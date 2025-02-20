use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use indexmap::{IndexMap, indexmap};
use serde::{Deserialize, Deserializer, de};

use crate::content::{ContentFileConfig, ProcessContent};

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_content_dir")]
    content_dir: Utf8PathBuf,
    #[serde(default = "default_asset_dir")]
    asset_dir: Utf8PathBuf,
    #[serde(default = "default_template_dir")]
    template_dir: Utf8PathBuf,
    #[serde(default = "default_sublime_dir")]
    sublime_dir: Utf8PathBuf,
    #[serde(default = "default_output_dir")]
    output_dir: Utf8PathBuf,

    #[serde(default, rename = "content")]
    pub content_file_settings: ContentFileSettings,
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,

    /// The path to the config file.
    ///
    /// Populated by [`read_config`][crate::read_config] after deserialization.
    #[serde(skip, default)]
    pub(crate) path: Utf8PathBuf,
}

impl Config {
    /// Get the path to the config file, as passed to
    /// [`read_config`][crate::read_config].
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn content_dir(&self) -> Utf8PathBuf {
        self.project_root().join(&self.content_dir)
    }

    pub fn asset_dir(&self) -> Utf8PathBuf {
        self.project_root().join(&self.asset_dir)
    }

    pub fn template_dir(&self) -> Utf8PathBuf {
        self.project_root().join(&self.template_dir)
    }

    pub fn sublime_dir(&self) -> Utf8PathBuf {
        self.project_root().join(&self.sublime_dir)
    }

    pub fn output_dir(&self) -> Utf8PathBuf {
        self.project_root().join(&self.output_dir)
    }

    pub fn set_output_dir(&mut self, value: Utf8PathBuf) {
        self.output_dir = value;
    }

    /// Get the "project root", that is the parent directory of the config file.
    ///
    /// Content, asset and output directory paths from the config are treated
    /// as relative to this.
    fn project_root(&self) -> &Utf8Path {
        assert_ne!(self.path, "", "config path must be set at this point");
        self.path.parent().expect("config path must have a parent")
    }
}

fn default_content_dir() -> Utf8PathBuf {
    "content".into()
}

fn default_asset_dir() -> Utf8PathBuf {
    "theme/assets".into()
}

fn default_template_dir() -> Utf8PathBuf {
    "theme/templates".into()
}

fn default_sublime_dir() -> Utf8PathBuf {
    "theme/sublime".into()
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
    ) -> impl DoubleEndedIterator<Item = &ContentFileConfig> {
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
