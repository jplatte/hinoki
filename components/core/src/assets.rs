use std::{
    collections::BTreeMap,
    io::{self, BufReader, BufWriter, Read, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err::{self as fs, File};
use indexmap::IndexMap;
use rayon::iter::{IntoParallelRefIterator as _, ParallelIterator as _};
use serde::Serialize;
use tracing::{error, instrument, warn};

use self::file_config::{AssetFileConfig, ProcessContent};
use crate::{
    build::OutputDirManager, config::Config, frontmatter::parse_frontmatter, metadata::metadata_env,
};

mod file_config;

pub(crate) struct AssetsProcessor<'c, 's, 'sc> {
    metadata_env: minijinja::Environment<'static>,
    render_scope: &'s rayon::Scope<'sc>,
    cx: &'c AssetsProcessorContext<'c>,
}

impl<'c: 'sc, 's, 'sc> AssetsProcessor<'c, 's, 'sc> {
    pub(crate) fn new(
        render_scope: &'s rayon::Scope<'sc>,
        cx: &'c AssetsProcessorContext<'c>,
    ) -> Self {
        let metadata_env = metadata_env();
        Self { metadata_env, render_scope, cx }
    }

    pub(crate) fn run(&self) -> anyhow::Result<()> {
        self.process_assets_dir(&self.cx.assets_dir, WriteOutput::Yes)?;
        Ok(())
    }

    pub(crate) fn dump(&self) -> anyhow::Result<()> {
        let metadata = self.process_assets_dir(&self.cx.assets_dir, WriteOutput::No)?;
        println!("{metadata:#?}");

        Ok(())
    }

    fn process_assets_dir(
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

        let subdirs = subdirs
            .par_iter()
            .map(|path| {
                let file_name = path
                    .file_name()
                    .expect("read_dir iterator only yields entries with a file name part")
                    .to_owned();
                let dir_meta = self.process_assets_dir(path, write_output)?;

                Ok((file_name, dir_meta))
            })
            .collect::<anyhow::Result<BTreeMap<_, _>>>()?;

        let mut idx = 0;
        let files = files.iter().try_fold(Vec::new(), |mut v, path| {
            if let Some(file) = self
                .process_content_file(path.clone(), write_output)
                .with_context(|| format!("processing `{path}`"))?
            {
                idx += 1;
                v.push(file);
            }

            anyhow::Ok(v)
        })?;

        Ok(DirectoryMetadata { subdirs, files })
    }

    #[instrument(skip_all, fields(?content_path))]
    fn process_content_file(
        &self,
        content_path: Utf8PathBuf,
        write_output: WriteOutput,
    ) -> anyhow::Result<Option<FileMetadata>> {
        let source_path =
            content_path.strip_prefix("content/").context("invalid content_path")?.to_owned();

        let mut input_file = BufReader::new(File::open(&content_path)?);

        let frontmatter = parse_frontmatter(&mut input_file)?;
        let file_meta = self.file_metadata(source_path.clone(), frontmatter)?;

        if let WriteOutput::Yes = write_output {
            self.render_file(file_meta.clone(), input_file, content_path)?;
        }

        Ok(Some(file_meta))
    }

    fn file_metadata(
        &self,
        source_path: Utf8PathBuf,
        frontmatter: AssetFileConfig,
    ) -> anyhow::Result<FileMetadata> {
        // for defaults in
        // self.ctx.config.file_config_defaults.for_path(&source_path).rev() {
        //     frontmatter.apply_defaults(defaults);
        // }

        let source_file_stem = source_path.file_stem().expect("path must have a file name");
        let mut metadata_ctx =
            MetadataContext { source_path: &source_path, source_file_stem, slug: None };

        let slug = self
            .expand_metadata_tpl(frontmatter.slug, &metadata_ctx)
            .context("expanding slug template")?
            .unwrap_or_else(|| source_file_stem.to_owned());

        // Make slug available for path templates
        metadata_ctx.slug = Some(&slug);

        let path = match self.expand_metadata_tpl(frontmatter.path, &metadata_ctx)? {
            Some(path) => path
                .strip_prefix('/')
                .context("paths in frontmatter and defaults must begin with '/'")?
                .into(),
            None => source_path.clone(),
        };

        Ok(FileMetadata {
            slug,
            path,
            extra: frontmatter.extra,
            process_content: frontmatter.process_content,
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
        input_file: BufReader<File>,
        content_path: Utf8PathBuf,
    ) -> anyhow::Result<()> {
        let ctx = self.cx;
        let span = tracing::Span::current();

        self.render_scope.spawn(move |_| {
            let _guard = span.enter();

            if let Err(e) = render(file_meta, input_file, ctx, content_path) {
                error!("{e:#}");
                ctx.did_error.store(true, Ordering::Relaxed);
            }
        });

        Ok(())
    }
}

pub(crate) struct AssetsProcessorContext<'a> {
    config: &'a Config,
    assets_dir: Utf8PathBuf,
    output_dir_mgr: &'a OutputDirManager,
    pub(crate) did_error: AtomicBool,
}

impl<'a> AssetsProcessorContext<'a> {
    pub(crate) fn new(config: &'a Config, output_dir_mgr: &'a OutputDirManager) -> Self {
        let assets_dir = config.asset_dir();
        Self { config, assets_dir, output_dir_mgr, did_error: AtomicBool::new(false) }
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
    pub subdirs: BTreeMap<String, DirectoryMetadata>,
    pub files: Vec<FileMetadata>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FileMetadata {
    pub slug: String,
    pub path: Utf8PathBuf,
    #[serde(default)]
    pub extra: IndexMap<String, toml::Value>,

    // further data from frontmatter that should be printed in dump-metadata
    // but not passed to the template as `page.*`
    #[serde(skip)]
    pub process_content: Option<ProcessContent>,
}

fn render(
    file_meta: FileMetadata,
    mut input_file: BufReader<File>,
    ctx: &AssetsProcessorContext<'_>,
    content_path: Utf8PathBuf,
) -> anyhow::Result<()> {
    let output_path = ctx.output_path(&file_meta.path, &content_path)?;
    let mut output_file = BufWriter::new(File::create(output_path)?);

    // Don't buffer file contents in memory if no content processing is needed.
    if file_meta.process_content.is_none() {
        io::copy(&mut input_file, &mut output_file)?;
        return Ok(());
    }

    let mut content = String::new();
    input_file.read_to_string(&mut content)?;

    output_file.write_all(content.as_bytes())?;

    Ok(())
}

#[derive(Serialize)]
struct MetadataContext<'a> {
    source_path: &'a Utf8Path,
    source_file_stem: &'a str,
    slug: Option<&'a str>,
}

#[derive(Clone, Copy)]
enum WriteOutput {
    Yes,
    No,
}
