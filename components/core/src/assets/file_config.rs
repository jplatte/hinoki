use indexmap::{map::Entry as IndexMapEntry, IndexMap};
use serde::Deserialize;
use toml::map::Entry as TomlMapEntry;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssetFileConfig {
    /// What kind of processing should be done on the content, if any.
    pub process_content: Option<ProcessContent>,

    /// Custom rendered path for this page.
    pub path: Option<String>,

    /// Custom slug for this page, to replace the file basename.
    pub slug: Option<String>,

    /// Arbitrary additional user-defined data.
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,
}

impl AssetFileConfig {
    pub(crate) fn apply_defaults(&mut self, defaults: &AssetFileConfig) {
        if self.process_content.is_none() {
            self.process_content = defaults.process_content;
        }
        if self.path.is_none() {
            self.path = defaults.path.clone();
        }
        if self.slug.is_none() {
            self.slug = defaults.slug.clone();
        }
        apply_extra_defaults(&mut self.extra, &defaults.extra);
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
    // CompileSass,
    // CompileScss,
    // CompileTypescript,
}
