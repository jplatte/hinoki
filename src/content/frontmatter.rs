use std::io::{BufRead, ErrorKind, Seek};

use anyhow::Context as _;
use camino::Utf8PathBuf;
use indexmap::{map::Entry as IndexMapEntry, IndexMap};
use serde::Deserialize;
use toml::map::Entry as TomlMapEntry;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frontmatter {
    /// If set to `true`, this page will only be included in the output if
    /// building in dev mode.
    pub draft: Option<bool>,

    /// Path of the template to use for this page.
    ///
    /// Relative to the `theme/templates` directory.
    pub template: Option<Utf8PathBuf>,

    /// What kind of processing should be done on the content, if any.
    pub process_content: Option<ProcessContent>,

    /// Syntax highlighting theme for markdown code blocks.
    pub syntax_highlight_theme: Option<String>,

    /// Custom rendered path for this page.
    pub path: Option<String>,

    /// Page title.
    pub title: Option<String>,

    /// Page date.
    pub date: Option<String>,

    /// Custom slug for this page, to replace the file basename.
    pub slug: Option<String>,

    /// Arbitrary additional user-defined data.
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,
}

impl Frontmatter {
    pub(crate) fn apply_defaults(&mut self, defaults: &Frontmatter) {
        if self.draft.is_none() {
            self.draft = defaults.draft;
        }
        if self.template.is_none() {
            self.template = defaults.template.clone();
        }
        if self.process_content.is_none() {
            self.process_content = defaults.process_content;
        }
        if self.syntax_highlight_theme.is_none() {
            self.syntax_highlight_theme = defaults.syntax_highlight_theme.clone();
        }
        if self.path.is_none() {
            self.path = defaults.path.clone();
        }
        if self.title.is_none() {
            self.title = defaults.title.clone();
        }
        if self.date.is_none() {
            self.date = defaults.date.clone();
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
    MarkdownToHtml,
}

/// Looks for TOML frontmatter in the given reader and parses it if found.
///
/// If the input does not start with a frontmatter delimiter (line of `+++` with
/// optional trailing whitespace), returns `Ok(None)`. If the frontmatter
/// delimiter is found, parses all the lines between that one and the next one
/// found. If successful, the input will be advanced such that the remaining
/// content after the frontmatter can be processed from it.
pub(crate) fn parse_frontmatter(input: impl BufRead + Seek) -> anyhow::Result<Frontmatter> {
    // Read at most 256 bytes at once. Avoids loading lots of irrelevant data
    // into memory for binary files.
    let mut limited = input.take(256);

    macro_rules! bail_default {
        () => {{
            let mut input = limited.into_inner();
            input.rewind()?;
            return Ok(Frontmatter::default());
        }};
    }

    let mut buf = String::new();
    if let Err(e) = limited.read_line(&mut buf) {
        match e.kind() {
            // Invalid UTF-8
            ErrorKind::InvalidData => bail_default!(),
            _ => return Err(e.into()),
        }
    }

    if buf.trim_end() != "+++" {
        bail_default!();
    }

    // If frontmatter delimiter was found, don't limit reading anymore.
    let mut input = limited.into_inner();
    buf.clear();
    loop {
        input.read_line(&mut buf)?;
        if buf.lines().next_back().map_or(false, |l| l.trim_end() == "+++") {
            let frontmatter_end_idx = buf.rfind("+++").expect("already found once");
            buf.truncate(frontmatter_end_idx);
            break;
        }
    }

    toml::from_str(&buf).context("parsing frontmatter")
}

#[cfg(test)]
mod tests {}
