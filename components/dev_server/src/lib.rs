use std::{
    fmt,
    net::{Ipv6Addr, SocketAddr},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

use camino::Utf8Path;
use fs_err as fs;
use hinoki_cli::ServeArgs;
use hinoki_core::{
    build::Build,
    config::{Config, Inputs},
};
use hyper_util::service::TowerToHyperService;
use tempfile::tempdir;
use tower_http::services::ServeDir;
use tracing::{error, info};

pub fn run(config: Config, args: ServeArgs) -> ExitCode {
    let res = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the Runtime")
        .block_on(run_inner(config, args));

    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run_inner(mut config: Config, args: ServeArgs) -> anyhow::Result<()> {
    let output_dir = tempdir()?;
    config.set_output_dir(output_dir.path().to_owned().try_into()?);

    let build = Build::new(config, true);
    let begin = Instant::now();
    build.run();
    info!("Built site in {}", FormatDuration(begin.elapsed()));

    let config = build.config().clone();
    let _watch_guard = start_watch(build)?;
    serve(&config, args).await?;

    Ok(())
}

/// Start file notification watcher.
///
/// Dropping the returned value stops the watcher thread.
fn start_watch(build: Build) -> anyhow::Result<impl Drop> {
    use notify::{
        EventKind, RecursiveMode,
        event::{CreateKind, ModifyKind},
    };
    use notify_debouncer_full::{DebounceEventResult, new_debouncer};

    const DEBOUNCE_DURATION: Duration = Duration::from_millis(100);

    #[rustfmt::skip] // buggy, can remove when Inputs gets another field
    let Inputs {
        mut project_root,
        config_file,
        content_dir,
        asset_dir,
        template_dir,
        sublime_dir,
    } = build.config().inputs();

    if project_root == "" {
        // If the config path is only a filename, `parent()` returns an empty path.
        // We can't pass that to `fs::canonicalize`.
        project_root = ".".into()
    }
    let project_root_canon = fs::canonicalize(project_root)?;

    let mut debouncer = new_debouncer(DEBOUNCE_DURATION, None, {
        let project_root_canon = project_root_canon.clone();
        move |res: DebounceEventResult| match res {
            Err(errors) => {
                for error in errors {
                    error!("notify error: {error}");
                }
            }
            Ok(mut events) => {
                events.retain_mut(|ev| {
                    match &ev.kind {
                        EventKind::Access(_)
                        | EventKind::Create(CreateKind::Folder)
                        | EventKind::Modify(ModifyKind::Metadata(_)) => return false,
                        EventKind::Any
                        | EventKind::Create(_)
                        | EventKind::Modify(_)
                        | EventKind::Remove(_)
                        | EventKind::Other => {}
                    };

                    ev.paths.retain(|path| {
                        let rel_path = match path.strip_prefix(&project_root_canon) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("notify event path error: {e}");
                                return false;
                            }
                        };

                        *rel_path.as_os_str() == *config_file
                            || rel_path.starts_with(&content_dir)
                            || rel_path.starts_with(&asset_dir)
                            || rel_path.starts_with(&template_dir)
                            || rel_path.starts_with(&sublime_dir)
                    });

                    !ev.paths.is_empty()
                });

                if !events.is_empty() {
                    let begin = Instant::now();
                    build.run();
                    info!("Rebuilt site in {}", FormatDuration(begin.elapsed()));
                }
            }
        }
    })?;

    debouncer.watch(project_root_canon, RecursiveMode::Recursive)?;

    Ok(debouncer)
}

async fn serve(config: &Config, args: ServeArgs) -> anyhow::Result<()> {
    let url = format!("http://localhost:{}", args.port);
    info!("Starting development server on {url}");

    let addr = SocketAddr::from((Ipv6Addr::LOCALHOST, args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    if args.open
        && let Err(err) = open::that(url)
    {
        error!("Failed to open site: {err}");
    }

    let output_dir: Arc<Utf8Path> = Arc::from(&*config.output_dir());
    loop {
        let (socket, _remote_addr) = listener.accept().await?;

        let output_dir = Arc::clone(&output_dir);
        tokio::spawn(async move {
            let socket = hyper_util::rt::TokioIo::new(socket);
            let service = TowerToHyperService::new(ServeDir::new(&*output_dir));

            if let Err(err) =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(socket, service)
                    .await
            {
                error!("Failed to serve connection: {err:#}");
            }
        });
    }
}

struct FormatDuration(Duration);

impl fmt::Display for FormatDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let duration = self.0;
        let total_secs = duration.as_secs();
        if let hours @ 1.. = total_secs / 3600 {
            let minutes = total_secs / 60 % 60;
            return write!(f, "{hours}h {minutes}min");
        }
        if let minutes @ 1.. = total_secs / 60 {
            let secs = total_secs % 60;
            return write!(f, "{minutes}min {secs}s");
        }
        if total_secs >= 10 {
            return write!(f, "{total_secs}s");
        }

        let subsec_micros = duration.subsec_micros();
        let millis = subsec_micros / 1000;
        if total_secs > 0 {
            let first_decimal = millis / 100;
            return write!(f, "{total_secs}.{first_decimal}s");
        }
        if millis >= 10 {
            return write!(f, "{millis}ms");
        }
        if millis > 0 {
            let first_decimal = (subsec_micros % 1000) / 100;
            return write!(f, "{millis}.{first_decimal}ms");
        }

        // Getting below 10µs is unrealistic, so no need for extra branches
        write!(f, "{subsec_micros}µs")
    }
}
