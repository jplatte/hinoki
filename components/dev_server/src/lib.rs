use std::{
    net::{Ipv6Addr, SocketAddr},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use camino::Utf8Path;
use fs_err as fs;
use hinoki_core::{build::build, Config};
use hyper_util::service::TowerToHyperService;
use tempfile::tempdir;
use tower_http::services::ServeDir;
use tracing::{error, info};

pub fn run(config: Config) -> ExitCode {
    let res = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the Runtime")
        .block_on(run_inner(config));

    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run_inner(mut config: Config) -> anyhow::Result<()> {
    let output_dir = tempdir()?;
    config.output_dir = output_dir.path().to_owned().try_into()?;
    build(&config, true);

    let _watch_guard = start_watch(&config)?;
    serve(&config).await?;
    Ok(())
}

/// Start file notification watcher.
///
/// Dropping the returned value stops the watcher thread.
fn start_watch(config: &Config) -> anyhow::Result<impl Drop> {
    use notify::{
        event::{CreateKind, ModifyKind},
        EventKind, RecursiveMode, Watcher,
    };
    use notify_debouncer_full::{new_debouncer, DebounceEventResult};

    const DEBOUNCE_DURATION: Duration = Duration::from_millis(100);

    let current_dir = fs::canonicalize(".")?;

    let mut debouncer = new_debouncer(DEBOUNCE_DURATION, None, {
        let config = config.clone();
        let current_dir = current_dir.clone();
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
                        let rel_path = match path.strip_prefix(&current_dir) {
                            Ok(p) => p,
                            Err(e) => {
                                error!("notify event path error: {e}");
                                return false;
                            }
                        };

                        rel_path.starts_with(&config.path)
                            || rel_path.starts_with("content")
                            || rel_path.starts_with("theme")
                    });

                    !ev.paths.is_empty()
                });

                if !events.is_empty() {
                    build(&config, true);
                }
            }
        }
    })?;

    debouncer.watcher().watch(current_dir.as_ref(), RecursiveMode::Recursive)?;

    Ok(debouncer)
}

async fn serve(config: &Config) -> anyhow::Result<()> {
    info!("Starting development server on http://localhost:8000");

    let addr = SocketAddr::from((Ipv6Addr::LOCALHOST, 8000));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let output_dir: Arc<Utf8Path> = Arc::from(&*config.output_dir);
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
