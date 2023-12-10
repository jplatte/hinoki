use once_cell::sync::OnceCell;
use pulldown_cmark::{html::push_html, Options, Parser};

use super::syntax_highlighting::SyntaxHighlighter;

pub(crate) fn markdown_to_html(
    content: &str,
    syntax_highlighter: &OnceCell<SyntaxHighlighter>,
    syntax_highlight_theme: Option<&str>,
) -> anyhow::Result<String> {
    let parser = Parser::new_ext(content, Options::ENABLE_FOOTNOTES);
    let mut html_buf = String::new();

    #[cfg(feature = "syntax-highlighting")]
    let syntax_highlighter = syntax_highlighter.get_or_try_init(SyntaxHighlighter::new)?;

    #[cfg(feature = "syntax-highlighting")]
    if let Some(theme) = syntax_highlight_theme.or_else(|| syntax_highlighter.theme()) {
        let with_highlighting = syntax_highlighter.highlight(parser, theme)?;
        push_html(&mut html_buf, with_highlighting);
    } else {
        push_html(&mut html_buf, parser);
    }

    Ok(html_buf)
}
