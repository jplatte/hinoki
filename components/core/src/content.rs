use std::{
    collections::BTreeMap,
    io::{self, BufReader, BufWriter, Read, Seek, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use file_config::FileConfigDatetime;
use fs_err::{self as fs, File};
use indexmap::IndexMap;
use minijinja::context;
use rayon::iter::{IntoParallelRefIterator as _, ParallelIterator as _};
use serde::Serialize;
use smallvec::SmallVec;
use tracing::{error, instrument, warn};

use crate::{
    build::OutputDirManager,
    config::Config,
    frontmatter::parse_frontmatter,
    metadata::metadata_env,
    template::context::{
        serialize_hinoki_cx, DirectoryContext, GlobalContext, HinokiContext, RenderContext,
        TemplateContext,
    },
    util::HinokiDatetime,
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

        let dir_cx = DirectoryContext::new(subdirs);
        let mut idx = 0;
        // FIXME: Is it possible to make some sort of Flatten FromIterator
        // adapter that combines with the Result FromIterator impl such that
        // this doesn't need to be an explicit fold?
        let files = files.iter().try_fold(Vec::new(), |mut v, path| {
            v.extend(
                self.process_content_file(path, &mut idx, dir_cx.clone(), write_output)
                    .with_context(|| format!("processing `{path}`"))?,
            );

            anyhow::Ok(v)
        })?;

        dir_cx.set_files(files);
        Ok(dir_cx.into_metadata())
    }

    #[instrument(skip_all, fields(?content_path))]
    fn process_content_file(
        &self,
        content_path: &Utf8Path,
        idx: &mut usize,
        dir_cx: DirectoryContext,
        write_output: WriteOutput,
    ) -> anyhow::Result<SmallVec<[FileMetadata; 1]>> {
        let source_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(content_path)?);

        let frontmatter = parse_frontmatter(&mut input_file)?;
        let mut all_file_meta =
            self.all_file_metadata(source_path.clone(), idx, dir_cx, frontmatter)?;
        if !self.cx.include_drafts {
            all_file_meta.retain(|file_meta| !file_meta.draft);
        }

        if let WriteOutput::No = write_output {
            return Ok(all_file_meta);
        }

        match all_file_meta.clone().into_inner() {
            // We want to produce exactly one output file.
            //
            // Reuse the already-opened input file.
            Ok([file_meta]) => {
                self.render_file(file_meta, input_file, content_path.to_owned())?;
            }
            // We want to produce zero or multiple output files.
            //
            // Get the input file position and reopen the file at that position
            // for every render_file call.
            //
            // FIXME: This opens the file one more time than necessary, what's
            // a convenient way around that?
            //
            // FIXME: On linux, can open /proc/self/fd/NUM to be persistent
            // against live modifications and maybe other shenanigans. Also
            // likely marginally better for perf. See this article:
            // https://blog.gnoack.org/post/proc-fd-is-not-dup/
            Err(all_file_meta) => {
                let pos = input_file
                    .stream_position()
                    .context("failed to get end of frontmatter file position")?;
                drop(input_file);

                for file_meta in all_file_meta {
                    let mut input_file = BufReader::new(File::open(content_path)?);
                    input_file
                        .seek_relative(pos as _)
                        .context("failed to seek over frontmatter")?;
                    self.render_file(file_meta, input_file, content_path.to_owned())?;
                }
            }
        }

        Ok(all_file_meta)
    }

    fn all_file_metadata(
        &self,
        source_path: Utf8PathBuf,
        idx: &mut usize,
        dir_cx: DirectoryContext,
        mut frontmatter: ContentFileConfig,
    ) -> anyhow::Result<SmallVec<[FileMetadata; 1]>> {
        #[derive(Serialize)]
        pub(crate) struct RepeatContext {
            #[serde(rename = "$hinoki_cx", serialize_with = "serialize_hinoki_cx")]
            hinoki_cx: Arc<HinokiContext>,
        }

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

        let make_hinoki_cx = |idx| {
            HinokiContext::new(
                self.cx.template_global_cx.clone(),
                dir_cx.to_owned(),
                RenderContext::new(
                    idx,
                    #[cfg(feature = "syntax-highlighting")]
                    frontmatter.syntax_highlight_theme.clone(),
                ),
            )
        };

        if let Some(repeat_expr) = &frontmatter.repeat {
            let repeat_val = self
                .cx
                .template_env
                .compile_expression(repeat_expr)
                .context("failed to compile repeat expression")?
                .eval(RepeatContext { hinoki_cx: make_hinoki_cx(None) })
                .context("failed to evaluate repeat expression")?;

            let iter = repeat_val.try_iter().context("repeat value is not iterable")?;
            iter.map(|item| {
                let repeat = Some(Repeat { item });
                self.file_metadata(&source_path, idx, &frontmatter, make_hinoki_cx, repeat)
            })
            .collect()
        } else {
            let meta = self.file_metadata(&source_path, idx, &frontmatter, make_hinoki_cx, None)?;
            Ok(SmallVec::from_elem(meta, 1))
        }
    }

    fn file_metadata(
        &self,
        source_path: &Utf8Path,
        idx: &mut usize,
        frontmatter: &ContentFileConfig,
        make_hinoki_cx: impl Fn(Option<usize>) -> Arc<HinokiContext>,
        repeat: Option<Repeat>,
    ) -> anyhow::Result<FileMetadata> {
        let source_dir = match source_path.parent() {
            Some(parent) => {
                if parent == "" {
                    parent
                } else {
                    &Utf8PathBuf::from("/").join(parent)
                }
            }
            None => Utf8Path::new(""),
        };
        let source_file_stem = source_path.file_stem().expect("path must have a file name");

        let mut metadata_cx = MetadataContext {
            source_dir,
            source_file_stem,
            slug: None,
            title: None,
            date: None,
            repeat: repeat.as_ref(),
        };

        let slug = self
            .expand_metadata_tpl(frontmatter.slug.as_deref(), &metadata_cx)
            .context("expanding slug template")?
            .unwrap_or_else(|| source_file_stem.to_owned());
        let title = self
            .expand_metadata_tpl(frontmatter.title.as_deref(), &metadata_cx)
            .context("expanding title template")?;
        let date = match &frontmatter.date {
            Some(FileConfigDatetime::Bare(dt)) => Some(*dt),
            Some(FileConfigDatetime::String(s)) => self
                .expand_metadata_tpl(Some(s), &metadata_cx)
                .context("expanding date template")?
                .filter(|s| !s.is_empty())
                .map(|s| s.parse())
                .transpose()
                .context("parsing date field")?,
            None => None,
        };

        // Make slug, title and date available for path templates
        metadata_cx.slug = Some(&slug);
        metadata_cx.title = title.as_deref();
        metadata_cx.date = date;

        let path = match self.expand_metadata_tpl(frontmatter.path.as_deref(), &metadata_cx)? {
            Some(path) => path
                .strip_prefix('/')
                .context("paths in frontmatter and config.content must begin with '/'")?
                .into(),
            None => source_path.to_owned(),
        };

        let draft = frontmatter.draft.unwrap_or(false);
        let extra = frontmatter.extra.clone();
        let template = frontmatter.template.clone();
        let process = frontmatter.process;

        let hinoki_cx = make_hinoki_cx(Some(*idx));
        *idx += 1;

        Ok(FileMetadata {
            draft,
            slug,
            path,
            title,
            date,
            extra,
            repeat,
            template,
            process,
            hinoki_cx,
        })
    }

    fn expand_metadata_tpl(
        &self,
        maybe_value: Option<&str>,
        metadata_cx: &MetadataContext<'_>,
    ) -> anyhow::Result<Option<String>> {
        maybe_value
            .map(|value| {
                if value.contains('{') {
                    Ok(self.metadata_env.get_template(value)?.render(metadata_cx)?)
                } else {
                    Ok(value.to_owned())
                }
            })
            .transpose()
    }

    fn render_file(
        &self,
        file_meta: FileMetadata,
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

            if let Err(e) = render(file_meta, input_file, cx, content_path) {
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
    template_global_cx: GlobalContext,
    output_dir_mgr: &'a OutputDirManager,
    pub(crate) did_error: AtomicBool,
}

impl<'a> ContentProcessorContext<'a> {
    pub(crate) fn new(
        config: &'a Config,
        include_drafts: bool,
        template_env: minijinja::Environment<'a>,
        output_dir_mgr: &'a OutputDirManager,
        template_global_cx: GlobalContext,
    ) -> Self {
        Self {
            config,
            include_drafts,
            template_env,
            template_global_cx,
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
    pub date: Option<HinokiDatetime>,
    pub repeat: Option<Repeat>,
    pub extra: IndexMap<String, toml::Value>,

    // further data from frontmatter that should be printed in dump-metadata
    // but not passed to the template as `page.*`
    #[serde(skip)]
    pub template: Option<Utf8PathBuf>,
    #[serde(skip)]
    pub process: Option<ProcessContent>,
    #[serde(skip)]
    pub hinoki_cx: Arc<HinokiContext>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Repeat {
    /// The current item.
    item: minijinja::Value,
    // TODO: length, current index, properties of previous and next like path
    // TODO: maybe this struct should actually be a custom minijinja Object?
}

fn render(
    file_meta: FileMetadata,
    mut input_file: BufReader<File>,
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

    let hinoki_cx = &file_meta.hinoki_cx;

    #[cfg(feature = "markdown")]
    if let Some(ProcessContent::MarkdownToHtml) = file_meta.process {
        content = markdown_to_html(&content, hinoki_cx)?;
    }

    if let Some(template) = template {
        let extra = &cx.config.extra;
        let cx =
            TemplateContext { content, page: &file_meta, config: context! { extra }, hinoki_cx };

        template.render_to_write(cx, output_file)?;
    } else {
        output_file.write_all(content.as_bytes())?;
    }

    Ok(())
}

#[derive(Serialize)]
struct MetadataContext<'a> {
    source_dir: &'a Utf8Path,
    source_file_stem: &'a str,
    slug: Option<&'a str>,
    title: Option<&'a str>,
    date: Option<HinokiDatetime>,
    repeat: Option<&'a Repeat>,
}

#[derive(Clone, Copy)]
enum WriteOutput {
    Yes,
    No,
}
