use std::io::{BufRead, ErrorKind};

use anyhow::Context as _;
use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frontmatter {
    /// If set to `true`, this page will only be included in the output if
    /// building in dev mode.
    #[serde(default)]
    pub draft: bool,

    /// Path of the template to use for this page.
    ///
    /// Relative to the `templates` directory.
    pub template: Option<Utf8PathBuf>,

    /// What kind of processing should be done on the content, if any.
    pub process_content: Option<ProcessContent>,

    /// Custom rendered path for this page.
    ///
    /// If it ends in `/`, `index.html` is implicitly appended.
    pub path: Option<Utf8PathBuf>,

    /// Alias paths for this page.
    ///
    /// Every path in this list will be generated as another page in the output,
    /// rendered with the builtin `alias.html` template.
    #[serde(default)]
    pub aliases: Vec<Utf8PathBuf>,

    /// Page title.
    pub title: Option<String>,

    /// Page date.
    pub date: Option<DateTime<Utc>>,

    /// Custom slug for this page, to replace the file basename.
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
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
pub(crate) fn parse_frontmatter(input: impl BufRead) -> Result<Option<Frontmatter>, anyhow::Error> {
    let mut buf = String::new();

    // Read at most 256 bytes at once. Avoids loading lots of irrelevant data
    // into memory for binary files.
    let mut limited = input.take(256);
    if let Err(e) = limited.read_line(&mut buf) {
        let result = match e.kind() {
            // Invalid UTF-8
            ErrorKind::InvalidData => Ok(None),
            _ => Err(e.into()),
        };

        return result;
    }

    if buf.trim_end() != "+++" {
        return Ok(None);
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
