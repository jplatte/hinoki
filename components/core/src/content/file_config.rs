use camino::Utf8PathBuf;
use indexmap::{map::Entry as IndexMapEntry, IndexMap};
use serde::Deserialize;
use toml::map::Entry as TomlMapEntry;

use crate::util::HinokiDatetime;

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentFileConfig {
    /// If set to `true`, this page will only be included in the output if
    /// building in dev mode.
    pub draft: Option<bool>,

    /// Path of the template to use for this page.
    ///
    /// Relative to the `theme/templates` directory.
    pub template: Option<Utf8PathBuf>,

    /// What kind of processing should be done on the content, if any.
    pub process: Option<ProcessContent>,

    /// Syntax highlighting theme for markdown code blocks.
    pub syntax_highlight_theme: Option<String>,

    /// Custom rendered path for this page.
    pub path: Option<String>,

    /// Page title.
    pub title: Option<String>,

    /// Page date.
    pub date: Option<FileConfigDatetime>,

    /// Custom slug for this page, to replace the file basename.
    pub slug: Option<String>,

    /// Render this page once for each item in the iterator.
    ///
    /// The string must be a minijinja expression that evaluates to an iterator.
    ///
    /// For example: `get_files("directory") | chunks(10)`.
    pub repeat: Option<String>,

    /// Arbitrary additional user-defined data.
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum FileConfigDatetime {
    Bare(HinokiDatetime),
    String(String),
}

impl ContentFileConfig {
    pub(crate) fn apply_glob_config(&mut self, config: &ContentFileConfig) {
        if self.draft.is_none() {
            self.draft = config.draft;
        }
        if self.template.is_none() {
            self.template = config.template.clone();
        }
        if self.process.is_none() {
            self.process = config.process;
        }
        if self.syntax_highlight_theme.is_none() {
            self.syntax_highlight_theme = config.syntax_highlight_theme.clone();
        }
        if self.path.is_none() {
            self.path = config.path.clone();
        }
        if self.title.is_none() {
            self.title = config.title.clone();
        }
        if self.date.is_none() {
            self.date = config.date.clone();
        }
        if self.slug.is_none() {
            self.slug = config.slug.clone();
        }
        if self.repeat.is_none() {
            self.repeat = config.repeat.clone();
        }
        apply_extra_defaults(&mut self.extra, &config.extra);
    }
}

fn apply_extra_defaults(
    target: &mut IndexMap<String, toml::Value>,
    source: &IndexMap<String, toml::Value>,
) {
    for (key, value) in source {
        match target.entry(key.to_owned()) {
            IndexMapEntry::Occupied(mut entry) => {
                apply_inner_extra_defaults(entry.get_mut(), value);
            }
            IndexMapEntry::Vacant(entry) => {
                entry.insert(value.clone());
            }
        }
    }
}

fn apply_inner_extra_defaults(target: &mut toml::Value, source: &toml::Value) {
    let toml::Value::Table(target) = target else { return };
    let toml::Value::Table(source) = source else { return };

    for (key, value) in source {
        match target.entry(key.to_owned()) {
            TomlMapEntry::Occupied(mut entry) => {
                apply_inner_extra_defaults(entry.get_mut(), value);
            }
            TomlMapEntry::Vacant(entry) => {
                entry.insert(value.clone());
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcessContent {
    MarkdownToHtml,
}
