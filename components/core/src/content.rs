use std::{
    collections::BTreeMap,
    io::{self, BufReader, BufWriter, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err::{self as fs, File};
use indexmap::IndexMap;
use minijinja::context;
use rayon::iter::{IntoParallelRefIterator as _, ParallelIterator as _};
use serde::Serialize;
use time::{format_description::well_known::Iso8601, Date};
use tracing::{error, instrument, warn};

use crate::{
    build::OutputDirManager,
    config::Config,
    frontmatter::parse_frontmatter,
    metadata::metadata_env,
    template::context::{HinokiContext, TemplateContext},
};

mod file_config;
#[cfg(feature = "markdown")]
mod markdown;
#[cfg(feature = "syntax-highlighting")]
mod syntax_highlighting;

pub(crate) use self::file_config::{ContentFileConfig, ProcessContent};
#[cfg(feature = "markdown")]
pub(crate) use self::markdown::markdown_to_html;
#[cfg(feature = "syntax-highlighting")]
pub(crate) use self::syntax_highlighting::{LazySyntaxHighlighter, SyntaxHighlighter};

pub(crate) struct ContentProcessor<'c, 's, 'sc> {
    // FIXME: args, template_env, syntax_highlighter (in cx) plus render_scope
    // only actually needed for building, not for dumping. Abstract using a
    // trait an abstract build vs. dump behavior that way instead of internal
    // branching?
    metadata_env: minijinja::Environment<'static>,
    render_scope: &'s rayon::Scope<'sc>,
    cx: &'c ContentProcessorContext<'c>,
}

impl<'c: 'sc, 's, 'sc> ContentProcessor<'c, 's, 'sc> {
    pub(crate) fn new(
        render_scope: &'s rayon::Scope<'sc>,
        cx: &'c ContentProcessorContext<'c>,
    ) -> Self {
        let metadata_env = metadata_env();
        Self { metadata_env, render_scope, cx }
    }

    pub(crate) fn run(&self) -> anyhow::Result<()> {
        self.process_content_dir("content/".into(), WriteOutput::Yes)?;
        Ok(())
    }

    pub(crate) fn dump(&self) -> anyhow::Result<()> {
        let metadata = self.process_content_dir("content/".into(), WriteOutput::No)?;
        println!("{metadata:#?}");

        Ok(())
    }

    fn process_content_dir(
        &self,
        dir: &Utf8Path,
        write_output: WriteOutput,
    ) -> anyhow::Result<DirectoryMetadata> {
        let mut subdirs = Vec::new();
        let mut files = Vec::new();

        for res in fs::read_dir(dir)? {
            let entry = res?;
            let file_type = entry.file_type()?;
            let Ok(utf8_path) = Utf8PathBuf::from_path_buf(entry.path()) else {
                warn!("Skipping non-utf8 file `{}`", entry.path().display());
                continue;
            };

            if file_type.is_dir() {
                subdirs.push(utf8_path);
            } else {
                files.push(utf8_path);
            }
        }

        // First, process subdirectories (operate depth-first), such that they
        // are available for the minijinja context of the `pages`.
        let subdirs = Arc::new(
            subdirs
                .par_iter()
                .map(|path| {
                    let file_name = path
                        .file_name()
                        .expect("read_dir iterator only yields entries with a file name part")
                        .to_owned();
                    let dir_meta = self.process_content_dir(path, write_output)?;

                    Ok((file_name, dir_meta))
                })
                .collect::<anyhow::Result<BTreeMap<_, _>>>()?,
        );

        let files_oncelock = Arc::new(OnceLock::new());
        let mut idx = 0;
        let files = files.iter().try_fold(Vec::new(), |mut v, path| {
            let hinoki_cx = HinokiContext::new(
                #[cfg(feature = "syntax-highlighting")]
                self.cx.syntax_highlighter.clone(),
                files_oncelock.clone(),
                subdirs.clone(),
                idx,
            );

            if let Some(file) = self
                .process_content_file(path.clone(), hinoki_cx, write_output)
                .with_context(|| format!("processing `{path}`"))?
            {
                idx += 1;
                v.push(file);
            }

            anyhow::Ok(v)
        })?;

        files_oncelock.set(files).unwrap(); // only set from here
        Ok(DirectoryMetadata { subdirs, files: files_oncelock })
    }

    #[instrument(skip_all, fields(?content_path))]
    fn process_content_file(
        &self,
        content_path: Utf8PathBuf,
        mut hinoki_cx: HinokiContext,
        write_output: WriteOutput,
    ) -> anyhow::Result<Option<FileMetadata>> {
        let source_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(&content_path)?);

        let frontmatter = parse_frontmatter(&mut input_file)?;
        let file_meta = self.file_metadata(source_path.clone(), frontmatter)?;
        if !self.cx.include_drafts && file_meta.draft {
            return Ok(None);
        }

        if let WriteOutput::Yes = write_output {
            #[cfg(feature = "syntax-highlighting")]
            {
                hinoki_cx.syntax_highlight_theme = file_meta.syntax_highlight_theme.clone();
            }
            self.render_file(file_meta.clone(), hinoki_cx, input_file, content_path)?;
        }

        Ok(Some(file_meta))
    }

    fn file_metadata(
        &self,
        source_path: Utf8PathBuf,
        mut frontmatter: ContentFileConfig,
    ) -> anyhow::Result<FileMetadata> {
        for config in self.cx.config.content_file_settings.for_path(&source_path).rev() {
            frontmatter.apply_glob_config(config);
        }

        #[cfg(not(feature = "syntax-highlighting"))]
        if frontmatter.syntax_highlight_theme.is_some() {
            warn!(
                "syntax highlighting was requested, but hinoki
                 was compiled without support for syntax highlighting"
            );
        }

        let source_file_stem = source_path.file_stem().expect("path must have a file name");
        let mut metadata_cx = MetadataContext {
            source_path: &source_path,
            source_file_stem,
            slug: None,
            title: None,
            date: None,
        };

        let slug = self
            .expand_metadata_tpl(frontmatter.slug, &metadata_cx)
            .context("expanding slug template")?
            .unwrap_or_else(|| source_file_stem.to_owned());
        let title = self
            .expand_metadata_tpl(frontmatter.title, &metadata_cx)
            .context("expanding title template")?;
        let date = self
            .expand_metadata_tpl(frontmatter.date, &metadata_cx)
            .context("expanding date template")?
            .filter(|s| !s.is_empty())
            .map(|s| Date::parse(&s, &Iso8601::DATE))
            .transpose()
            .context("parsing date")?;

        // Make slug, title and date available for path templates
        metadata_cx.slug = Some(&slug);
        metadata_cx.title = title.as_deref();
        metadata_cx.date = date.as_ref();

        let path = match self.expand_metadata_tpl(frontmatter.path, &metadata_cx)? {
            Some(path) => path
                .strip_prefix('/')
                .context("paths in frontmatter and config.content must begin with '/'")?
                .into(),
            None => source_path.clone(),
        };

        Ok(FileMetadata {
            draft: frontmatter.draft.unwrap_or(false),
            slug,
            path,
            title,
            date,
            extra: frontmatter.extra,
            template: frontmatter.template,
            process: frontmatter.process,
            syntax_highlight_theme: frontmatter.syntax_highlight_theme,
        })
    }

    fn expand_metadata_tpl(
        &self,
        maybe_value: Option<String>,
        metadata_cx: &MetadataContext<'_>,
    ) -> anyhow::Result<Option<String>> {
        maybe_value
            .map(|value| {
                if value.contains('{') {
                    Ok(self.metadata_env.get_template(&value)?.render(metadata_cx)?)
                } else {
                    Ok(value)
                }
            })
            .transpose()
    }

    fn render_file(
        &self,
        file_meta: FileMetadata,
        hinoki_cx: HinokiContext,
        input_file: BufReader<File>,
        content_path: Utf8PathBuf,
    ) -> anyhow::Result<()> {
        #[cfg(not(feature = "markdown"))]
        if let Some(ProcessContent::MarkdownToHtml) = file_meta.process {
            anyhow::bail!(
                "hinoki was compiled without support for markdown.\
                 Please recompile with the 'markdown' feature enabled."
            );
        }

        let cx = self.cx;
        let span = tracing::Span::current();

        self.render_scope.spawn(move |_| {
            let _guard = span.enter();

            if let Err(e) = render(file_meta, input_file, hinoki_cx, cx, content_path) {
                error!("{e:#}");
                cx.did_error.store(true, Ordering::Relaxed);
            }
        });

        Ok(())
    }
}

pub(crate) struct ContentProcessorContext<'a> {
    config: &'a Config,
    include_drafts: bool,
    template_env: minijinja::Environment<'a>,
    #[cfg(feature = "syntax-highlighting")]
    syntax_highlighter: LazySyntaxHighlighter,
    output_dir_mgr: &'a OutputDirManager,
    pub(crate) did_error: AtomicBool,
}

impl<'a> ContentProcessorContext<'a> {
    pub(crate) fn new(
        config: &'a Config,
        include_drafts: bool,
        template_env: minijinja::Environment<'a>,
        output_dir_mgr: &'a OutputDirManager,
        #[cfg(feature = "syntax-highlighting")] syntax_highlighter: LazySyntaxHighlighter,
    ) -> Self {
        Self {
            config,
            include_drafts,
            template_env,
            #[cfg(feature = "syntax-highlighting")]
            syntax_highlighter,
            output_dir_mgr,
            did_error: AtomicBool::new(false),
        }
    }

    fn output_path(
        &self,
        file_path: &Utf8Path,
        content_path: &Utf8Path,
    ) -> anyhow::Result<Utf8PathBuf> {
        self.output_dir_mgr.output_path(file_path, content_path)
    }
}

#[derive(Debug)]
pub(crate) struct DirectoryMetadata {
    pub subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
    pub files: Arc<OnceLock<Vec<FileMetadata>>>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FileMetadata {
    pub draft: bool,
    pub slug: String,
    pub path: Utf8PathBuf,
    pub title: Option<String>,
    pub date: Option<Date>,
    pub extra: IndexMap<String, toml::Value>,

    // further data from frontmatter that should be printed in dump-metadata
    // but not passed to the template as `page.*`
    #[serde(skip)]
    pub template: Option<Utf8PathBuf>,
    #[serde(skip)]
    pub process: Option<ProcessContent>,
    #[serde(skip)]
    pub syntax_highlight_theme: Option<String>,
}

fn render(
    file_meta: FileMetadata,
    mut input_file: BufReader<File>,
    hinoki_cx: HinokiContext,
    cx: &ContentProcessorContext<'_>,
    content_path: Utf8PathBuf,
) -> anyhow::Result<()> {
    let template = file_meta
        .template
        .as_ref()
        .map(|tpl| cx.template_env.get_template(tpl.as_str()))
        .transpose()?;

    let output_path = cx.output_path(&file_meta.path, &content_path)?;
    let mut output_file = BufWriter::new(File::create(output_path)?);

    // Don't buffer file contents in memory if no templating or content
    // processing is needed.
    if template.is_none() && file_meta.process.is_none() {
        io::copy(&mut input_file, &mut output_file)?;
        return Ok(());
    }

    let mut content = String::new();
    input_file.read_to_string(&mut content)?;

    #[cfg(feature = "markdown")]
    if let Some(ProcessContent::MarkdownToHtml) = file_meta.process {
        let syntax_highlight_theme = file_meta.syntax_highlight_theme.as_deref();
        content = markdown_to_html(
            &content,
            #[cfg(feature = "syntax-highlighting")]
            &cx.syntax_highlighter,
            #[cfg(feature = "syntax-highlighting")]
            syntax_highlight_theme,
        )?;
    }

    if let Some(template) = template {
        let extra = &cx.config.extra;
        let cx = TemplateContext {
            content,
            page: &file_meta,
            config: context! { extra },
            hinoki_cx: Arc::new(hinoki_cx),
        };

        template.render_to_write(cx, output_file)?;
    } else {
        output_file.write_all(content.as_bytes())?;
    }

    Ok(())
}

#[derive(Serialize)]
struct MetadataContext<'a> {
    source_path: &'a Utf8Path,
    source_file_stem: &'a str,
    slug: Option<&'a str>,
    title: Option<&'a str>,
    date: Option<&'a Date>,
}

#[derive(Clone, Copy)]
enum WriteOutput {
    Yes,
    No,
}
