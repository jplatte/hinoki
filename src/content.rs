use std::{
    collections::BTreeMap,
    io::{self, BufReader, BufWriter, Read, Write},
    process::ExitCode,
    sync::{mpsc, Arc, OnceLock},
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
use tracing::{error, instrument, trace, warn};
use walkdir::WalkDir;

use self::frontmatter::parse_frontmatter;
#[cfg(feature = "syntax-highlighting")]
use self::syntax_highlighting::SyntaxHighlighter;
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

pub(crate) fn build(args: BuildArgs, config: Config) -> ExitCode {
    fn build_inner(
        args: BuildArgs,
        defaults: Defaults,
        build_dir_mgr: &BuildDirManager,
        error_tx: mpsc::Sender<anyhow::Error>,
    ) -> anyhow::Result<()> {
        let alloc = Herd::new();
        let template_env = load_templates(&alloc)?;
        let ctx = ContentProcessorContext::new(args, defaults, template_env, build_dir_mgr);
        rayon::scope(|scope| ContentProcessor::new(scope, error_tx, &ctx).run())
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

    let (error_tx, error_rx) = mpsc::channel();
    let build_dir_mgr = BuildDirManager::new(config.output_dir);

    let (r1, r2) = rayon::join(
        || build_inner(args, config.defaults, &build_dir_mgr, error_tx),
        || copy_static_files(&build_dir_mgr),
    );

    let errors = r1.err().into_iter().chain(r2.err()).chain(error_rx.iter());

    let mut exit = ExitCode::SUCCESS;
    for e in errors {
        exit = anyhow_exit(e);
    }

    exit
}

pub(crate) fn dump(config: Config) -> ExitCode {
    let (error_tx, error_rx) = mpsc::channel();
    let build_dir_mgr = BuildDirManager::new("".into());
    let ctx = ContentProcessorContext::new(
        BuildArgs { include_drafts: true },
        config.defaults,
        minijinja::Environment::empty(),
        &build_dir_mgr,
    );

    let res = rayon::scope(|scope| ContentProcessor::new(scope, error_tx, &ctx).dump());
    assert!(error_rx.recv().is_err());

    match res {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => anyhow_exit(e),
    }
}

fn anyhow_exit(error: anyhow::Error) -> ExitCode {
    error!("{error:#}");
    ExitCode::FAILURE
}

struct ContentProcessor<'c, 's, 'sc> {
    // FIXME: args, template_env, syntax_highlighter (in ctx) plus render_scope
    // only actually needed for building, not for dumping. Abstract using a
    // trait an abstract build vs. dump behavior that way instead of internal
    // branching?
    metadata_env: minijinja::Environment<'static>,
    render_scope: &'s rayon::Scope<'sc>,
    error_tx: mpsc::Sender<anyhow::Error>,
    ctx: &'c ContentProcessorContext<'c>,
}

impl<'c: 'sc, 's, 'sc> ContentProcessor<'c, 's, 'sc> {
    fn new(
        render_scope: &'s rayon::Scope<'sc>,
        error_tx: mpsc::Sender<anyhow::Error>,
        ctx: &'c ContentProcessorContext<'c>,
    ) -> Self {
        let metadata_env = metadata_env();
        Self { metadata_env, render_scope, error_tx, ctx }
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
        let file_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(&content_path)?);

        let frontmatter = parse_frontmatter(&mut input_file)?;
        let file_meta = self.file_metadata(file_path.clone(), frontmatter)?;
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
        file_path: Utf8PathBuf,
        mut frontmatter: Frontmatter,
    ) -> anyhow::Result<FileMetadata> {
        #[derive(Serialize)]
        struct MetadataContext<'a> {
            slug: &'a str,
            // TODO: More fields
        }

        for defaults in self.ctx.defaults.for_path(&file_path).rev() {
            frontmatter.apply_defaults(defaults);
        }

        let slug = frontmatter.slug.unwrap_or_else(|| {
            trace!("Generating slug for `{file_path}`");
            let slug = file_path.file_stem().expect("path must have a file name").to_owned();
            trace!("Slug for `{file_path}` is `{slug}`");
            slug
        });

        let metadata_ctx = MetadataContext { slug: &slug };

        let path = match frontmatter.path {
            // If path comes from frontmatter or defaults, apply templating
            Some(path) => self.metadata_env.get_template(&path)?.render(&metadata_ctx)?.into(),
            // Otherwise, use the path relative to content
            None => file_path,
        };

        let title = frontmatter
            .title
            .map(|title| self.metadata_env.get_template(&title)?.render(&metadata_ctx))
            .transpose()?;

        #[cfg(not(feature = "syntax-highlighting"))]
        if frontmatter.syntax_highlight_theme.is_some() {
            warn!(
                "syntax highlighting was requested, but hinoki
                 was compiled without support for syntax highlighting"
            );
        }

        Ok(FileMetadata {
            draft: frontmatter.draft.unwrap_or(false),
            slug,
            path,
            title,
            // TODO: allow extracting from file name?
            date: frontmatter.date,
            template: frontmatter.template,
            process_content: frontmatter.process_content,
            syntax_highlight_theme: frontmatter.syntax_highlight_theme,
        })
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
        let error_tx = self.error_tx.clone();

        self.render_scope.spawn(move |_| {
            let _guard = span.enter();

            if let Err(e) = render(file_meta, input_file, functions, ctx, content_path) {
                error_tx.send(e).unwrap();
            }
        });

        Ok(())
    }
}

fn metadata_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::empty();

    env.set_loader(|tpl| Ok(Some(tpl.to_owned())));
    env.set_syntax(minijinja::Syntax {
        block_start: "{%".into(),
        block_end: "%}".into(),
        variable_start: "{".into(),
        variable_end: "}".into(),
        comment_start: "{#".into(),
        comment_end: "#}".into(),
    })
    .expect("custom minijinja syntax is valid");

    env.add_filter("default", minijinja::filters::default);
    env.add_filter("first", minijinja::filters::first);
    env.add_filter("join", minijinja::filters::join);
    env.add_filter("last", minijinja::filters::last);
    env.add_filter("replace", minijinja::filters::replace);
    env.add_filter("reverse", minijinja::filters::reverse);
    env.add_filter("sort", minijinja::filters::sort);
    env.add_filter("trim", minijinja::filters::trim);

    env
}

struct ContentProcessorContext<'a> {
    args: BuildArgs,
    defaults: Defaults,
    template_env: minijinja::Environment<'a>,
    #[cfg(feature = "syntax-highlighting")]
    syntax_highlighter: OnceCell<SyntaxHighlighter>,
    build_dir_mgr: &'a BuildDirManager,
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

#[derive(Clone, Copy)]
enum WriteOutput {
    Yes,
    No,
}
