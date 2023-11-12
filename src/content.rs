use std::{
    collections::BTreeMap,
    io::{BufReader, Read},
    sync::{mpsc, Arc, OnceLock},
};

use anyhow::{format_err, Context as _};
use bumpalo_herd::Herd;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err::{self as fs, File};
use itertools::{Either, Itertools};
use minijinja::context;
#[cfg(feature = "syntax-highlighting")]
use once_cell::sync::OnceCell;
use rayon::iter::{IntoParallelRefIterator as _, ParallelBridge as _, ParallelIterator as _};
use serde::Serialize;
use tracing::{debug, instrument, trace, warn};
use walkdir::WalkDir;

#[cfg(feature = "syntax-highlighting")]
use self::syntax_highlighting::SyntaxHighlighter;
use self::{
    frontmatter::{parse_frontmatter, Frontmatter},
    metadata::FileMetadata,
};
use crate::{
    cli::BuildArgs,
    config::Config,
    template::{self, functions},
};

mod frontmatter;
mod metadata;
#[cfg(feature = "syntax-highlighting")]
mod syntax_highlighting;

pub(crate) use self::{
    frontmatter::ProcessContent,
    metadata::{AssetMetadata, DirectoryMetadata, PageMetadata},
};

// FIXME: Collect errors instead of returning only the first

pub(crate) fn build(args: BuildArgs, config: Config) -> anyhow::Result<()> {
    let alloc = Herd::new();
    let template_env = load_templates(&alloc)?;
    let syntax_highlighter = OnceCell::new();

    fs::create_dir_all(&config.output_dir)?;

    let (error_tx, error_rx) = mpsc::channel();
    rayon::scope(|scope| {
        ContentProcessor::new(
            args,
            config,
            &template_env,
            scope,
            error_tx,
            #[cfg(feature = "syntax-highlighting")]
            &syntax_highlighter,
        )
        .run()
    })?;

    if let Ok(e) = error_rx.recv() {
        return Err(e);
    }

    Ok(())
}

pub(crate) fn dump(config: Config) -> anyhow::Result<()> {
    let (error_tx, error_rx) = mpsc::channel();
    let template_env = minijinja::Environment::new();
    let syntax_highlighter = OnceCell::new();
    rayon::scope(|scope| {
        ContentProcessor::new(
            BuildArgs { include_drafts: true },
            config,
            &template_env,
            scope,
            error_tx,
            #[cfg(feature = "syntax-highlighting")]
            &syntax_highlighter,
        )
        .dump()
    })?;

    assert!(error_rx.recv().is_err());

    Ok(())
}

struct ContentProcessor<'a, 's, 'sc> {
    // FIXME: args, template_env, render_scope only actually needed for
    // building, not for dumping. Make a trait for those two instead of
    // branching internally?
    args: BuildArgs,
    config: Config,
    template_env: &'a minijinja::Environment<'a>,
    metadata_env: minijinja::Environment<'static>,
    render_scope: &'s rayon::Scope<'sc>,
    error_tx: mpsc::Sender<anyhow::Error>,
    #[cfg(feature = "syntax-highlighting")]
    syntax_highlighter: &'a OnceCell<SyntaxHighlighter>,
}

fn load_templates(alloc: &Herd) -> anyhow::Result<minijinja::Environment<'_>> {
    struct TemplateSource<'b> {
        /// Path relative to the template directory
        rel_path: &'b str,
        /// File contents
        source: &'b str,
    }

    let mut template_env = template::environment();

    let (template_source_tx, template_source_rx) = mpsc::channel();
    let read_templates = move || {
        WalkDir::new("theme/templates/").into_iter().par_bridge().try_for_each_init(
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
                    utf8_path.strip_prefix("theme/templates/").context("invalid WalkDir item")?;

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

impl<'a: 'sc, 's, 'sc> ContentProcessor<'a, 's, 'sc> {
    fn new(
        args: BuildArgs,
        config: Config,
        template_env: &'a minijinja::Environment<'a>,
        render_scope: &'s rayon::Scope<'sc>,
        error_tx: mpsc::Sender<anyhow::Error>,
        #[cfg(feature = "syntax-highlighting")] syntax_highlighter: &'a OnceCell<SyntaxHighlighter>,
    ) -> Self {
        let mut metadata_env = minijinja::Environment::empty();
        metadata_env
            .set_syntax(minijinja::Syntax {
                block_start: "{%".into(),
                block_end: "%}".into(),
                variable_start: "{".into(),
                variable_end: "}".into(),
                comment_start: "{#".into(),
                comment_end: "#}".into(),
            })
            .expect("custom minijinja syntax is valid");
        metadata_env.set_loader(|tpl| Ok(Some(tpl.to_owned())));

        Self {
            args,
            config,
            template_env,
            metadata_env,
            render_scope,
            error_tx,
            #[cfg(feature = "syntax-highlighting")]
            syntax_highlighter,
        }
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
                .process_content_file(path, functions.clone(), write_output)
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
        content_path: &Utf8Path,
        functions: minijinja::Value,
        write_output: WriteOutput,
    ) -> anyhow::Result<Option<FileMetadata>> {
        let page_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(content_path)?);

        Ok(Some(match parse_frontmatter(&mut input_file)? {
            Some(frontmatter) => {
                let page_meta = self.page_metadata(page_path, frontmatter)?;
                if !self.args.include_drafts && page_meta.draft {
                    return Ok(None);
                }

                if let WriteOutput::Yes = write_output {
                    self.render_page(page_meta.clone(), functions, input_file)?;
                }

                FileMetadata::Page(page_meta)
            }
            None => {
                drop(input_file);

                FileMetadata::Asset(self.process_asset(write_output, page_path, content_path)?)
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

        for defaults in self.config.defaults.for_path(&page_path).rev() {
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
    ) -> anyhow::Result<()> {
        #[cfg(not(feature = "markdown"))]
        if let Some(ProcessContent::MarkdownToHtml) = page_meta.process_content {
            anyhow::bail!(
                "hinoki was compiled without support for markdown.\
                 Please recompile with the 'markdown' feature enabled."
            );
        }

        let template_env = self.template_env;
        let output_path = self.output_path(&page_meta.path);
        #[cfg(feature = "syntax-highlighting")]
        let syntax_highlighter = self.syntax_highlighter;

        let span = tracing::Span::current();
        let error_tx = self.error_tx.clone();

        self.render_scope.spawn(move |_| {
            let _guard = span.enter();

            if let Err(e) = render_page(
                template_env,
                page_meta,
                input_file,
                output_path,
                #[cfg(feature = "syntax-highlighting")]
                syntax_highlighter,
                functions,
            ) {
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
            let output_path = self.output_path(&page_path);

            debug!("copying file without frontmatter verbatim");
            // todo: keep track of dirs created to avoid extra create_dir_all's?
            fs::create_dir_all(output_path.parent().unwrap())?;
            fs::copy(content_path, output_path)?;
        }

        Ok(AssetMetadata::new(page_path))
    }

    fn output_path(&self, page_path: &Utf8Path) -> Utf8PathBuf {
        self.config.output_dir.join(page_path)
    }
}

fn render_page(
    template_env: &minijinja::Environment<'_>,
    page_meta: PageMetadata,
    mut input_file: BufReader<File>,
    output_path: Utf8PathBuf,
    #[cfg(feature = "syntax-highlighting")] syntax_highlighter: &OnceCell<SyntaxHighlighter>,
    functions: minijinja::Value,
) -> anyhow::Result<()> {
    let template = template_env.get_template(page_meta.template.as_str())?;

    let mut content = String::new();
    input_file.read_to_string(&mut content)?;

    fs::create_dir_all(output_path.parent().unwrap())?;
    let output_file = File::create(output_path)?;

    #[cfg(feature = "markdown")]
    if let Some(ProcessContent::MarkdownToHtml) = page_meta.process_content {
        use pulldown_cmark::{html::push_html, Options, Parser};

        let parser = Parser::new_ext(&content, Options::ENABLE_FOOTNOTES);
        let mut html_buf = String::new();

        #[cfg(feature = "syntax-highlighting")]
        let syntax_highlighter = syntax_highlighter.get_or_try_init(SyntaxHighlighter::new)?;

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
