#[cfg(feature = "markdown")]
pub(crate) fn markdown(state: &minijinja::State, input: &str) -> Result<String, minijinja::Error> {
    use crate::{content::markdown_to_html, template::context::MinijinjaStateExt as _};

    #[cfg(not(feature = "syntax-highlighting"))]
    let _ = state;

    #[cfg(feature = "syntax-highlighting")]
    let hinoki_cx = state.hinoki_cx()?;

    markdown_to_html(
        input,
        #[cfg(feature = "syntax-highlighting")]
        hinoki_cx.syntax_highlighter().map_err(|e| {
            minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
        })?,
        #[cfg(feature = "syntax-highlighting")]
        hinoki_cx.syntax_highlight_theme(),
    )
    .map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
}
