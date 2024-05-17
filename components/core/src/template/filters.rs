#[cfg(feature = "syntax-highlighting")]
use std::sync::Arc;

#[cfg(feature = "syntax-highlighting")]
use once_cell::sync::OnceCell;

#[cfg(feature = "syntax-highlighting")]
use crate::content::SyntaxHighlighter;

#[cfg(feature = "markdown")]
pub(crate) fn markdown(
    #[cfg(feature = "syntax-highlighting")] syntax_highlighter: Arc<OnceCell<SyntaxHighlighter>>,
) -> impl for<'a> Fn(&'a minijinja::State, &'a str) -> Result<String, minijinja::Error> {
    use crate::{content::markdown_to_html, template::context::MinijinjaStateExt as _};

    move |state, input| {
        #[cfg(not(feature = "syntax-highlighting"))]
        let _ = state;

        markdown_to_html(
            input,
            #[cfg(feature = "syntax-highlighting")]
            &syntax_highlighter,
            #[cfg(feature = "syntax-highlighting")]
            state.hinoki_cx()?.syntax_highlight_theme.as_deref(),
        )
        .map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
    }
}
