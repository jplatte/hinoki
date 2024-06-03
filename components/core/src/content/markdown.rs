use pulldown_cmark::{html::push_html, Options, Parser};

use crate::template::context::HinokiContext;

pub(crate) fn markdown_to_html(content: &str, hinoki_cx: &HinokiContext) -> anyhow::Result<String> {
    #[cfg(feature = "syntax-highlighting")]
    let syntax_highlighter = hinoki_cx.syntax_highlighter()?;

    let parser = Parser::new_ext(content, Options::ENABLE_FOOTNOTES);
    let mut html_buf = String::new();

    #[cfg(feature = "syntax-highlighting")]
    if let Some(theme) = hinoki_cx.syntax_highlight_theme().or_else(|| syntax_highlighter.theme()) {
        let with_highlighting = syntax_highlighter.highlight(parser, theme)?;
        push_html(&mut html_buf, with_highlighting);
    } else {
        push_html(&mut html_buf, parser);
    }

    Ok(html_buf)
}
