use std::{
    collections::BTreeMap,
    io::{self, BufReader, BufWriter, Read, Write},
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

use anyhow::Context as _;
use bumpalo_herd::Herd;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err::{self as fs, File};
use minijinja::context;
#[cfg(feature = "syntax-highlighting")]
use once_cell::sync::OnceCell;
use rayon::iter::{IntoParallelRefIterator as _, ParallelBridge as _, ParallelIterator as _};
use serde::Serialize;
use time::{format_description::well_known::Iso8601, Date};
use tracing::{error, instrument, warn};
use walkdir::WalkDir;

#[cfg(feature = "syntax-highlighting")]
use self::syntax_highlighting::SyntaxHighlighter;
use self::{frontmatter::parse_frontmatter, metadata::metadata_env};
use crate::{
    build::BuildDirManager,
    cli::BuildArgs,
    config::{Config, Defaults},
    template::{functions, load_templates},
};

mod frontmatter;
mod metadata;
#[cfg(feature = "syntax-highlighting")]
mod syntax_highlighting;

pub(crate) use self::{
    frontmatter::{Frontmatter, ProcessContent},
    metadata::{DirectoryMetadata, FileMetadata},
};

pub fn build(args: BuildArgs, config: Config) -> ExitCode {
    fn build_inner(
        args: BuildArgs,
        defaults: Defaults,
        build_dir_mgr: &BuildDirManager,
    ) -> anyhow::Result<bool> {
        let alloc = Herd::new();
        let template_env = load_templates(&alloc)?;
        let ctx = ContentProcessorContext::new(args, defaults, template_env, build_dir_mgr);
        rayon::scope(|scope| ContentProcessor::new(scope, &ctx).run())?;
        Ok(ctx.did_error.load(Ordering::Relaxed))
    }

    fn copy_static_files(build_dir_mgr: &BuildDirManager) -> anyhow::Result<()> {
        WalkDir::new("theme/static/").into_iter().par_bridge().try_for_each(|entry| {
            let entry = entry?;
            if entry.file_type().is_dir() {
                return Ok(());
            }

            let Some(utf8_path) = Utf8Path::from_path(entry.path()) else {
                warn!("Skipping non-utf8 file `{}`", entry.path().display());
                return Ok(());
            };

            let rel_path =
                utf8_path.strip_prefix("theme/static/").context("invalid WalkDir item")?;
            let output_path = build_dir_mgr.output_path(rel_path, utf8_path)?;

            fs::copy(utf8_path, output_path)?;
            Ok(())
        })
    }

    let build_dir_mgr = BuildDirManager::new(config.output_dir);

    let (r1, r2) = rayon::join(
        || build_inner(args, config.defaults, &build_dir_mgr),
        || copy_static_files(&build_dir_mgr),
    );

    match (r1, r2) {
        (Err(e1), Err(e2)) => {
            error!("{e1:#}");
            error!("{e2:#}");
            ExitCode::FAILURE
        }
        (Ok(_), Err(e)) | (Err(e), Ok(_)) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
        (Ok(true), Ok(())) => ExitCode::FAILURE,
        (Ok(false), Ok(())) => ExitCode::SUCCESS,
    }
}

pub fn dump(config: Config) -> ExitCode {
    let build_dir_mgr = BuildDirManager::new("".into());
    let ctx = ContentProcessorContext::new(
        BuildArgs { include_drafts: true },
        config.defaults,
        minijinja::Environment::empty(),
        &build_dir_mgr,
    );

    let res = rayon::scope(|scope| ContentProcessor::new(scope, &ctx).dump());
    assert!(!ctx.did_error.load(Ordering::Relaxed));

    match res {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

struct ContentProcessor<'c, 's, 'sc> {
    // FIXME: args, template_env, syntax_highlighter (in ctx) plus render_scope
    // only actually needed for building, not for dumping. Abstract using a
    // trait an abstract build vs. dump behavior that way instead of internal
    // branching?
    metadata_env: minijinja::Environment<'static>,
    render_scope: &'s rayon::Scope<'sc>,
    ctx: &'c ContentProcessorContext<'c>,
}

impl<'c: 'sc, 's, 'sc> ContentProcessor<'c, 's, 'sc> {
    fn new(render_scope: &'s rayon::Scope<'sc>, ctx: &'c ContentProcessorContext<'c>) -> Self {
        let metadata_env = metadata_env();
        Self { metadata_env, render_scope, ctx }
    }

    fn run(&self) -> anyhow::Result<()> {
        self.process_content_dir("content/".into(), WriteOutput::Yes)?;
        Ok(())
    }

    fn dump(&self) -> anyhow::Result<()> {
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
            let functions = context! {
                get_file => minijinja::Value::from_object(
                    functions::GetFile::new(files_oncelock.clone(), subdirs.clone(), idx),
                ),
                get_files => minijinja::Value::from(
                    functions::GetFiles::new(subdirs.clone()),
                ),
            };

            if let Some(file) = self
                .process_content_file(path.clone(), functions.clone(), write_output)
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
        functions: minijinja::Value,
        write_output: WriteOutput,
    ) -> anyhow::Result<Option<FileMetadata>> {
        let source_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(&content_path)?);

        let frontmatter = parse_frontmatter(&mut input_file)?;
        let file_meta = self.file_metadata(source_path.clone(), frontmatter)?;
        if !self.ctx.args.include_drafts && file_meta.draft {
            return Ok(None);
        }

        if let WriteOutput::Yes = write_output {
            self.render_file(file_meta.clone(), functions, input_file, content_path)?;
        }

        Ok(Some(file_meta))
    }

    fn file_metadata(
        &self,
        source_path: Utf8PathBuf,
        mut frontmatter: Frontmatter,
    ) -> anyhow::Result<FileMetadata> {
        for defaults in self.ctx.defaults.for_path(&source_path).rev() {
            frontmatter.apply_defaults(defaults);
        }

        #[cfg(not(feature = "syntax-highlighting"))]
        if frontmatter.syntax_highlight_theme.is_some() {
            warn!(
                "syntax highlighting was requested, but hinoki
                 was compiled without support for syntax highlighting"
            );
        }

        let source_file_stem = source_path.file_stem().expect("path must have a file name");
        let mut metadata_ctx = MetadataContext {
            source_path: &source_path,
            source_file_stem,
            slug: None,
            title: None,
            date: None,
        };

        let slug = self
            .expand_metadata_tpl(frontmatter.slug, &metadata_ctx)
            .context("expanding slug template")?
            .unwrap_or_else(|| source_file_stem.to_owned());
        let title = self
            .expand_metadata_tpl(frontmatter.title, &metadata_ctx)
            .context("expanding title template")?;
        let date = self
            .expand_metadata_tpl(frontmatter.date, &metadata_ctx)
            .context("expanding date template")?
            .filter(|s| !s.is_empty())
            .map(|s| Date::parse(&s, &Iso8601::DATE))
            .transpose()
            .context("parsing date")?;

        // Make slug, title and date available for path templates
        metadata_ctx.slug = Some(&slug);
        metadata_ctx.title = title.as_deref();
        metadata_ctx.date = date.as_ref();

        let path = match self.expand_metadata_tpl(frontmatter.path, &metadata_ctx)? {
            Some(path) => path
                .strip_prefix('/')
                .context("paths in frontmatter and defaults must begin with '/'")?
                .into(),
            None => source_path.clone(),
        };

        Ok(FileMetadata {
            draft: frontmatter.draft.unwrap_or(false),
            slug,
            path,
            title,
            date,
            template: frontmatter.template,
            process_content: frontmatter.process_content,
            syntax_highlight_theme: frontmatter.syntax_highlight_theme,
        })
    }

    fn expand_metadata_tpl(
        &self,
        maybe_value: Option<String>,
        metadata_ctx: &MetadataContext<'_>,
    ) -> anyhow::Result<Option<String>> {
        maybe_value
            .map(|value| {
                if value.contains('{') {
                    Ok(self.metadata_env.get_template(&value)?.render(metadata_ctx)?)
                } else {
                    Ok(value)
                }
            })
            .transpose()
    }

    fn render_file(
        &self,
        file_meta: FileMetadata,
        functions: minijinja::Value,
        input_file: BufReader<File>,
        content_path: Utf8PathBuf,
    ) -> anyhow::Result<()> {
        #[cfg(not(feature = "markdown"))]
        if let Some(ProcessContent::MarkdownToHtml) = file_meta.process_content {
            anyhow::bail!(
                "hinoki was compiled without support for markdown.\
                 Please recompile with the 'markdown' feature enabled."
            );
        }

        let ctx = self.ctx;
        let span = tracing::Span::current();

        self.render_scope.spawn(move |_| {
            let _guard = span.enter();

            if let Err(e) = render(file_meta, input_file, functions, ctx, content_path) {
                error!("{e:#}");
                ctx.did_error.store(true, Ordering::Relaxed);
            }
        });

        Ok(())
    }
}

struct ContentProcessorContext<'a> {
    args: BuildArgs,
    defaults: Defaults,
    template_env: minijinja::Environment<'a>,
    #[cfg(feature = "syntax-highlighting")]
    syntax_highlighter: OnceCell<SyntaxHighlighter>,
    build_dir_mgr: &'a BuildDirManager,
    did_error: AtomicBool,
}

impl<'a> ContentProcessorContext<'a> {
    fn new(
        args: BuildArgs,
        defaults: Defaults,
        template_env: minijinja::Environment<'a>,
        build_dir_mgr: &'a BuildDirManager,
    ) -> Self {
        Self {
            args,
            defaults,
            template_env,
            #[cfg(feature = "syntax-highlighting")]
            syntax_highlighter: OnceCell::new(),
            build_dir_mgr,
            did_error: AtomicBool::new(false),
        }
    }

    fn output_path(
        &self,
        file_path: &Utf8Path,
        content_path: &Utf8Path,
    ) -> anyhow::Result<Utf8PathBuf> {
        self.build_dir_mgr.output_path(file_path, content_path)
    }
}

fn render(
    file_meta: FileMetadata,
    mut input_file: BufReader<File>,
    functions: minijinja::Value,
    ctx: &ContentProcessorContext<'_>,
    content_path: Utf8PathBuf,
) -> anyhow::Result<()> {
    let template = file_meta
        .template
        .as_ref()
        .map(|tpl| ctx.template_env.get_template(tpl.as_str()))
        .transpose()?;

    let output_path = ctx.output_path(&file_meta.path, &content_path)?;
    let mut output_file = BufWriter::new(File::create(output_path)?);

    // Don't buffer file contents in memory if no templating or content
    // processing is needed.
    if template.is_none() && file_meta.process_content.is_none() {
        io::copy(&mut input_file, &mut output_file)?;
        return Ok(());
    }

    let mut content = String::new();
    input_file.read_to_string(&mut content)?;

    #[cfg(feature = "markdown")]
    if let Some(ProcessContent::MarkdownToHtml) = file_meta.process_content {
        use pulldown_cmark::{html::push_html, Options, Parser};

        let parser = Parser::new_ext(&content, Options::ENABLE_FOOTNOTES);
        let mut html_buf = String::new();

        #[cfg(feature = "syntax-highlighting")]
        let syntax_highlighter = ctx.syntax_highlighter.get_or_try_init(SyntaxHighlighter::new)?;

        #[cfg(feature = "syntax-highlighting")]
        if let Some(theme) =
            file_meta.syntax_highlight_theme.as_deref().or_else(|| syntax_highlighter.theme())
        {
            let with_highlighting = syntax_highlighter.highlight(parser, theme)?;
            push_html(&mut html_buf, with_highlighting);
        } else {
            push_html(&mut html_buf, parser);
        }

        content = html_buf;
    }

    if let Some(template) = template {
        let ctx = context! {
            content,
            page => &file_meta,
            ..functions
        };

        template.render_to_write(ctx, output_file)?;
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
