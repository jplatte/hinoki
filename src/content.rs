use std::{
    io::{BufReader, Read},
    sync::{mpsc, OnceLock},
};

use anyhow::{format_err, Context as _};
use bumpalo_herd::Herd;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err::{self as fs, File};
use itertools::{Either, Itertools};
use rayon::iter::{IntoParallelRefIterator as _, ParallelBridge as _, ParallelIterator as _};
use serde::Serialize;
use tracing::{debug, instrument, trace, warn};
use walkdir::WalkDir;

use crate::{cli::BuildArgs, config::Config, template};

use self::{
    frontmatter::{parse_frontmatter, Frontmatter},
    metadata::{AssetMetadata, DirectoryMetadata, FileMetadata, PageMetadata},
};

mod frontmatter;
mod metadata;

pub(crate) use self::frontmatter::ProcessContent;

pub(crate) fn build(args: BuildArgs, config: Config) -> anyhow::Result<()> {
    let alloc = Herd::new();
    let template_env = load_templates(&alloc)?;

    fs::create_dir_all("build")?;
    ContentProcessor::new(args, config, template_env).run()
}

pub(crate) fn dump(config: Config) -> anyhow::Result<()> {
    ContentProcessor::new(BuildArgs {}, config, minijinja::Environment::new()).dump()
}

struct ContentProcessor<'a> {
    args: BuildArgs,
    config: Config,
    template_env: minijinja::Environment<'a>,
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

impl<'a> ContentProcessor<'a> {
    fn new(args: BuildArgs, config: Config, template_env: minijinja::Environment<'a>) -> Self {
        Self { args, config, template_env }
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
        let subdirs = subdirs
            .par_iter()
            .map(|path| {
                let file_name = path
                    .file_name()
                    .expect("read_dir iterator only yields entries with a file name part")
                    .to_owned();
                let dir_meta = self.process_content_dir(path, write_output)?;

                Ok((file_name, dir_meta))
            })
            .collect::<anyhow::Result<_>>()?;

        let files: Vec<_> = files
            .par_iter()
            .map(|path| {
                self.process_content_file(path, write_output)
                    .with_context(|| format!("processing `{path}`"))
            })
            .collect::<anyhow::Result<_>>()?;

        let (pages, assets) = files.into_iter().partition_map(|file_meta| match file_meta {
            FileMetadata::Page(meta) => Either::Left(meta),
            FileMetadata::Asset(meta) => Either::Right(meta),
        });

        Ok(DirectoryMetadata { subdirs, pages, assets })
    }

    #[instrument(skip(self, write_output))]
    fn process_content_file(
        &self,
        content_path: &Utf8Path,
        write_output: WriteOutput,
    ) -> anyhow::Result<FileMetadata> {
        let page_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(content_path)?);

        Ok(match parse_frontmatter(&mut input_file)? {
            Some(frontmatter) => {
                let page_meta = self.page_metadata(page_path, frontmatter)?;

                if let WriteOutput::Yes = write_output {
                    self.render_page(&page_meta, input_file)?;
                }

                FileMetadata::Page(page_meta)
            }
            None => {
                drop(input_file);

                FileMetadata::Asset(self.process_asset(write_output, page_path, content_path)?)
            }
        })
    }

    fn page_metadata(
        &self,
        page_path: Utf8PathBuf,
        mut frontmatter: Frontmatter,
    ) -> anyhow::Result<PageMetadata> {
        for defaults in self.config.defaults.for_path(&page_path).rev() {
            frontmatter.apply_defaults(defaults);
        }

        Ok(PageMetadata {
            draft: frontmatter.draft.unwrap_or(false),
            slug: frontmatter.slug.unwrap_or_else(|| {
                trace!("Generating slug for `{page_path}`");
                let slug = page_path.file_stem().expect("path must have a file name").to_owned();
                trace!("Slug for `{page_path}` is `{slug}`");
                slug
            }),
            path: match frontmatter.path {
                // If path comes from frontmatter or defaults, apply templating
                Some(path) => self.render_frontmatter_template(path)?.into(),
                // Otherwise, use the path relative to content
                None => page_path,
            },
            title: frontmatter
                .title
                .map(|title| self.render_frontmatter_template(title))
                .transpose()?,
            // TODO: allow extracting from file name?
            date: frontmatter.date,
            template: frontmatter.template.context("no template specified")?,
            process_content: frontmatter.process_content,
        })
    }

    fn render_frontmatter_template(&self, template: String) -> anyhow::Result<String> {
        static META_ENVIRONMENT: OnceLock<minijinja::Environment<'static>> = OnceLock::new();
        let environment = META_ENVIRONMENT.get_or_init(|| {
            let mut environment = minijinja::Environment::empty();
            environment
                .set_syntax(minijinja::Syntax {
                    block_start: "{%".into(),
                    block_end: "%}".into(),
                    variable_start: "{".into(),
                    variable_end: "}".into(),
                    comment_start: "{#".into(),
                    comment_end: "#}".into(),
                })
                .expect("custom minijinja syntax is valid");
            environment.set_loader(|tpl| Ok(Some(tpl.to_owned())));
            environment
        });

        #[derive(Serialize)]
        struct Context {
            // TODO!
        }

        Ok(environment.get_template(&template)?.render(Context {})?)
    }

    fn render_page(
        &self,
        page_meta: &PageMetadata,
        mut input_file: BufReader<File>,
    ) -> Result<(), anyhow::Error> {
        #[cfg(not(feature = "markdown"))]
        if let Some(ProcessContent::MarkdownToHtml) = page_meta.process_content {
            anyhow::bail!(
                "hinoki was compiled without support for markdown.\
                 Please recompile with the 'markdown' feature enabled."
            );
        }

        let template = self.template_env.get_template(page_meta.template.as_str())?;

        let mut content = String::new();
        input_file.read_to_string(&mut content)?;

        #[cfg(feature = "markdown")]
        if let Some(ProcessContent::MarkdownToHtml) = page_meta.process_content {
            use pulldown_cmark::{html::push_html, Parser};

            let parser = Parser::new(&content);
            let mut html_buf = String::new();
            push_html(&mut html_buf, parser);

            content = html_buf;
        }

        let ctx = RenderContext { content: &content, page: page_meta };
        let output_file = File::create(self.output_path(&page_meta.path))?;
        template.render_to_write(ctx, output_file)?;

        Ok(())
    }

    fn process_asset(
        &self,
        write_output: WriteOutput,
        page_path: Utf8PathBuf,
        content_path: &Utf8Path,
    ) -> Result<AssetMetadata, anyhow::Error> {
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

#[derive(Clone, Copy)]
enum WriteOutput {
    Yes,
    No,
}

#[derive(Serialize)]
struct RenderContext<'a> {
    content: &'a str,
    page: &'a PageMetadata,
}
