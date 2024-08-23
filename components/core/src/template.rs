use std::sync::mpsc;

use anyhow::{format_err, Context as _};
use bumpalo_herd::Herd;
use camino::Utf8Path;
use fs_err::{self as fs};
use minijinja::UndefinedBehavior;
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use tracing::warn;
use walkdir::WalkDir;

pub(crate) mod context;
pub(crate) mod filters;
pub(crate) mod functions;

pub(crate) fn load_templates<'a>(
    template_dir: &Utf8Path,
    alloc: &'a Herd,
) -> anyhow::Result<minijinja::Environment<'a>> {
    struct TemplateSource<'b> {
        /// Path relative to the template directory
        rel_path: &'b str,
        /// File contents
        source: &'b str,
    }

    let mut template_env = environment();

    let (template_source_tx, template_source_rx) = mpsc::channel();
    let read_templates = move || {
        WalkDir::new(template_dir).into_iter().par_bridge().try_for_each_init(
            || alloc.get(),
            move |alloc, entry| {
                let entry = entry?;
                if entry.file_type().is_dir() {
                    return Ok(());
                }

                let Some(utf8_path) = Utf8Path::from_path(entry.path()) else {
                    warn!("Skipping non-utf8 file `{}`", entry.path().display());
                    return Ok(());
                };
                let rel_path =
                    utf8_path.strip_prefix(template_dir).context("invalid WalkDir item")?;

                let template_file_content = fs::read_to_string(utf8_path)?;

                template_source_tx
                    .send(TemplateSource {
                        rel_path: alloc.alloc_str(rel_path.as_str()),
                        source: alloc.alloc_str(&template_file_content),
                    })
                    .map_err(|_| {
                        // If the channel was closed by the receiving side, that
                        // implies an error in adding templates, which will be
                        // printed independently. Thus we don't need a good
                        // error message here, it will probably never be
                        // printed anyways.
                        //
                        // It is important to discard the original `SendError`
                        // though, which can't be converted to `anyhow::Error`
                        // because it's not `'static` (the compile errors from
                        // this are completely inscrutable and I only found out
                        // by experimenting).
                        format_err!("channel closed")
                    })?;

                anyhow::Ok(())
            },
        )
    };

    let template_env_ref = &mut template_env;
    let add_templates = move || {
        while let Ok(TemplateSource { rel_path, source }) = template_source_rx.recv() {
            template_env_ref.add_template(rel_path, source)?;
        }

        anyhow::Ok(())
    };

    let (read_templates_result, add_templates_result) = rayon::join(read_templates, add_templates);

    // Prioritize errors from add_templates, if it fails then read_templates
    // almost definitely also fails with a RecvError and the case of a
    // parallel I/O error is super rare and not very important.
    add_templates_result?;
    read_templates_result?;

    Ok(template_env)
}

fn environment<'a>() -> minijinja::Environment<'a> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    #[cfg(feature = "markdown")]
    env.add_filter("markdown", filters::markdown);
    env.add_function("get_file", functions::get_file);
    env.add_function("get_files", functions::get_files);
    env.add_function("load_data", functions::load_data);

    #[cfg(feature = "datetime")]
    {
        use minijinja_contrib::filters as contrib_filters;

        env.add_filter("dateformat", contrib_filters::dateformat);
        env.add_filter("datetimeformat", contrib_filters::datetimeformat);
        env.add_filter("timeformat", contrib_filters::timeformat);
    }

    env
}
