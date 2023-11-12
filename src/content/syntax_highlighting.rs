use std::collections::BTreeMap;

use anyhow::Context as _;
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Tag};
use syntect::{
    highlighting::{Theme, ThemeSet},
    html::highlighted_html_for_string,
    parsing::SyntaxSet,
};
use tracing::{error, warn};

pub(super) struct SyntaxHighlighter {
    syntaxset: SyntaxSet,
    themes: BTreeMap<String, Theme>,
}

impl SyntaxHighlighter {
    pub(super) fn new() -> anyhow::Result<SyntaxHighlighter> {
        let syntaxset = SyntaxSet::load_defaults_newlines();
        let themes = ThemeSet::load_from_folder("theme/sublime")
            .context("Loading syntax highlighting themes")?
            .themes;

        Ok(SyntaxHighlighter { syntaxset, themes })
    }

    /// If there is exactly one theme, returns it.
    pub(super) fn theme(&self) -> Option<&str> {
        if self.themes.len() == 1 {
            self.themes.keys().next().map(String::as_str)
        } else {
            None
        }
    }

    pub(super) fn highlight<'a>(
        &'a self,
        events: impl Iterator<Item = Event<'a>>,
        theme_name: &str,
    ) -> anyhow::Result<impl Iterator<Item = Event<'a>>> {
        let theme = self
            .themes
            .get(theme_name)
            .with_context(|| format!("theme `{theme_name}` not found"))?;

        // FIXME: Not optimal to look this up unconditionally, but not a serious problem
        // either.
        let plaintext_syntax = self.syntaxset.find_syntax_plain_text();

        let mut current_code_block_language = None;
        let mut current_code_block_contents = String::new();

        Ok(events.filter_map(move |event| match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))) => {
                current_code_block_language = Some(language);
                None
            }
            // FIXME: use if-let guard when stable (https://github.com/rust-lang/rust/issues/51114)
            ev @ Event::End(Tag::CodeBlock(_)) => match current_code_block_language.take() {
                Some(language) => {
                    let syntax =
                        self.syntaxset.find_syntax_by_token(&language).unwrap_or_else(|| {
                            warn!(?language, "no matching sublime syntax found");
                            plaintext_syntax
                        });

                    let code = &current_code_block_contents;
                    let highlight_result =
                        highlighted_html_for_string(code, &self.syntaxset, syntax, theme);

                    let event = match highlight_result {
                        Ok(html) => Event::Html(CowStr::from(html)),
                        Err(e) => {
                            error!("Failed to highlight code block: {e}");

                            // FIXME: Use flat_map with three events here instead
                            Event::Html(CowStr::from(format!("<code>{code}</code>")))
                        }
                    };

                    current_code_block_contents.clear();
                    Some(event)
                }
                None => Some(ev),
            },
            Event::Text(t) => {
                if current_code_block_language.is_some() {
                    current_code_block_contents.push_str(&t);
                    None
                } else {
                    Some(Event::Text(t))
                }
            }
            ev => Some(ev),
        }))
    }
}
