use pulldown_cmark::{html::push_html, Options, Parser};

#[cfg(feature = "syntax-highlighting")]
use super::syntax_highlighting::SyntaxHighlighter;

pub(crate) fn markdown_to_html(
    content: &str,
    #[cfg(feature = "syntax-highlighting")] syntax_highlighter: &SyntaxHighlighter,
    #[cfg(feature = "syntax-highlighting")] syntax_highlight_theme: Option<&str>,
) -> anyhow::Result<String> {
    let parser = Parser::new_ext(content, Options::ENABLE_FOOTNOTES);
    let mut html_buf = String::new();

    #[cfg(feature = "syntax-highlighting")]
    if let Some(theme) = syntax_highlight_theme.or_else(|| syntax_highlighter.theme()) {
        let with_highlighting = syntax_highlighter.highlight(parser, theme)?;
        push_html(&mut html_buf, with_highlighting);
    } else {
        push_html(&mut html_buf, parser);
    }

    Ok(html_buf)
}
