#[cfg(feature = "markdown")]
pub(crate) fn markdown(state: &minijinja::State, input: &str) -> Result<String, minijinja::Error> {
    use crate::{content::markdown_to_html, template::context::MinijinjaStateExt as _};

    #[cfg(not(feature = "syntax-highlighting"))]
    let _ = state;

    #[cfg(feature = "syntax-highlighting")]
    let hinoki_cx = state.hinoki_cx()?;

    markdown_to_html(input, &hinoki_cx)
        .map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
}
