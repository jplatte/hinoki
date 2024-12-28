use std::io::{BufRead, ErrorKind, Seek};

use anyhow::Context as _;
use serde::de::DeserializeOwned;

/// Looks for TOML frontmatter in the given reader and parses it if found.
///
/// If the input does not start with a frontmatter delimiter (line of `+++` with
/// optional trailing whitespace), returns `Ok(None)`. If the frontmatter
/// delimiter is found, parses all the lines between that one and the next one
/// found. If successful, the input will be advanced such that the remaining
/// content after the frontmatter can be processed from it.
pub(crate) fn parse_frontmatter<T>(input: impl BufRead + Seek) -> anyhow::Result<T>
where
    T: Default + DeserializeOwned,
{
    // Read at most 256 bytes at once. Avoids loading lots of irrelevant data
    // into memory for binary files.
    let mut limited = input.take(256);

    macro_rules! bail_default {
        () => {{
            let mut input = limited.into_inner();
            input.rewind()?;
            return Ok(T::default());
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
        if buf.lines().next_back().is_some_and(|l| l.trim_end() == "+++") {
            let frontmatter_end_idx = buf.rfind("+++").expect("already found once");
            buf.truncate(frontmatter_end_idx);
            break;
        }
    }

    toml::from_str(&buf).context("parsing frontmatter")
}
