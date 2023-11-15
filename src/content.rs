use std::{
    collections::BTreeMap,
    io::{BufReader, Read},
    process::ExitCode,
    sync::{mpsc, Arc, OnceLock},
};

use anyhow::Context as _;
use bumpalo_herd::Herd;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err::{self as fs, File};
use itertools::{Either, Itertools};
use minijinja::context;
#[cfg(feature = "syntax-highlighting")]
use once_cell::sync::OnceCell;
use rayon::iter::{IntoParallelRefIterator as _, ParallelBridge as _, ParallelIterator as _};
use serde::Serialize;
use tracing::{debug, error, instrument, trace, warn};
use walkdir::WalkDir;

#[cfg(feature = "syntax-highlighting")]
use self::syntax_highlighting::SyntaxHighlighter;
use self::{frontmatter::parse_frontmatter, metadata::FileMetadata};
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
    metadata::{AssetMetadata, DirectoryMetadata, PageMetadata},
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
    error!("{error}");
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

        let pages = Arc::new(OnceLock::new());
        let mut idx = 0;
        let files = files.iter().try_fold(Vec::new(), |mut v, path| {
            let functions = context! {
                get_page => minijinja::Value::from_object(
                    functions::GetPage::new(pages.clone(), subdirs.clone(), idx),
                ),
                get_pages => minijinja::Value::from(
                    functions::GetPages::new(subdirs.clone()),
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

        let (pages_vec, assets) = files.into_iter().partition_map(|file_meta| match file_meta {
            FileMetadata::Page(meta) => Either::Left(meta),
            FileMetadata::Asset(meta) => Either::Right(meta),
        });

        pages.set(pages_vec).unwrap(); // only set from here
        Ok(DirectoryMetadata { subdirs, pages, assets })
    }

    #[instrument(skip_all, fields(?content_path))]
    fn process_content_file(
        &self,
        content_path: Utf8PathBuf,
        functions: minijinja::Value,
        write_output: WriteOutput,
    ) -> anyhow::Result<Option<FileMetadata>> {
        let page_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(&content_path)?);

        Ok(Some(match parse_frontmatter(&mut input_file)? {
            Some(frontmatter) => {
                let page_meta = self.page_metadata(page_path, frontmatter)?;
                if !self.ctx.args.include_drafts && page_meta.draft {
                    return Ok(None);
                }

                if let WriteOutput::Yes = write_output {
                    self.render_page(page_meta.clone(), functions, input_file, content_path)?;
                }

                FileMetadata::Page(page_meta)
            }
            None => {
                drop(input_file);

                FileMetadata::Asset(self.process_asset(write_output, page_path, &content_path)?)
            }
        }))
    }

    fn page_metadata(
        &self,
        page_path: Utf8PathBuf,
        mut frontmatter: Frontmatter,
    ) -> anyhow::Result<PageMetadata> {
        #[derive(Serialize)]
        struct FrontmatterRenderContext<'a> {
            slug: &'a str,
            // TODO: More fields
        }

        for defaults in self.ctx.defaults.for_path(&page_path).rev() {
            frontmatter.apply_defaults(defaults);
        }

        let slug = frontmatter.slug.unwrap_or_else(|| {
            trace!("Generating slug for `{page_path}`");
            let slug = page_path.file_stem().expect("path must have a file name").to_owned();
            trace!("Slug for `{page_path}` is `{slug}`");
            slug
        });

        let frontmatter_ctx = FrontmatterRenderContext { slug: &slug };

        let path = match frontmatter.path {
            // If path comes from frontmatter or defaults, apply templating
            Some(path) => self.metadata_env.get_template(&path)?.render(&frontmatter_ctx)?.into(),
            // Otherwise, use the path relative to content
            None => page_path,
        };

        let title = frontmatter
            .title
            .map(|title| self.metadata_env.get_template(&title)?.render(&frontmatter_ctx))
            .transpose()?;

        #[cfg(not(feature = "syntax-highlighting"))]
        if frontmatter.syntax_highlight_theme.is_some() {
            warn!(
                "syntax highlighting was requested, but hinoki
                 was compiled without support for syntax highlighting"
            );
        }

        Ok(PageMetadata {
            draft: frontmatter.draft.unwrap_or(false),
            slug,
            path,
            title,
            // TODO: allow extracting from file name?
            date: frontmatter.date,
            template: frontmatter.template.context("no template specified")?,
            process_content: frontmatter.process_content,
            syntax_highlight_theme: frontmatter.syntax_highlight_theme,
        })
    }

    fn render_page(
        &self,
        page_meta: PageMetadata,
        functions: minijinja::Value,
        input_file: BufReader<File>,
        content_path: Utf8PathBuf,
    ) -> anyhow::Result<()> {
        #[cfg(not(feature = "markdown"))]
        if let Some(ProcessContent::MarkdownToHtml) = page_meta.process_content {
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

            if let Err(e) = render_page(page_meta, input_file, functions, ctx, content_path) {
                error_tx.send(e).unwrap();
            }
        });

        Ok(())
    }

    fn process_asset(
        &self,
        write_output: WriteOutput,
        page_path: Utf8PathBuf,
        content_path: &Utf8Path,
    ) -> anyhow::Result<AssetMetadata> {
        if let WriteOutput::Yes = write_output {
            let output_path = self.ctx.output_path(&page_path, content_path)?;

            debug!("copying file without frontmatter verbatim");
            fs::copy(content_path, output_path)?;
        }

        Ok(AssetMetadata::new(page_path))
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
        page_path: &Utf8Path,
        content_path: &Utf8Path,
    ) -> anyhow::Result<Utf8PathBuf> {
        self.build_dir_mgr.output_path(page_path, content_path)
    }
}

fn render_page(
    page_meta: PageMetadata,
    mut input_file: BufReader<File>,
    functions: minijinja::Value,
    ctx: &ContentProcessorContext<'_>,
    content_path: Utf8PathBuf,
) -> anyhow::Result<()> {
    let template = ctx.template_env.get_template(page_meta.template.as_str())?;

    let mut content = String::new();
    input_file.read_to_string(&mut content)?;

    let output_path = ctx.output_path(&page_meta.path, &content_path)?;
    let output_file = File::create(output_path)?;

    #[cfg(feature = "markdown")]
    if let Some(ProcessContent::MarkdownToHtml) = page_meta.process_content {
        use pulldown_cmark::{html::push_html, Options, Parser};

        let parser = Parser::new_ext(&content, Options::ENABLE_FOOTNOTES);
        let mut html_buf = String::new();

        #[cfg(feature = "syntax-highlighting")]
        let syntax_highlighter = ctx.syntax_highlighter.get_or_try_init(SyntaxHighlighter::new)?;

        #[cfg(feature = "syntax-highlighting")]
        if let Some(theme) =
            page_meta.syntax_highlight_theme.as_deref().or_else(|| syntax_highlighter.theme())
        {
            let with_highlighting = syntax_highlighter.highlight(parser, theme)?;
            push_html(&mut html_buf, with_highlighting);
        } else {
            push_html(&mut html_buf, parser);
        }

        content = html_buf;
    }

    let ctx = context! {
        content,
        page => &page_meta,
        ..functions
    };

    template.render_to_write(ctx, output_file)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum WriteOutput {
    Yes,
    No,
}
