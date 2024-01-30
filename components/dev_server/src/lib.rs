use std::{
    net::{Ipv6Addr, SocketAddr},
    process::ExitCode,
    sync::Arc,
};

use camino::Utf8Path;
use futures_util::future::try_join;
use hinoki_core::Config;
use hyper_util::service::TowerToHyperService;
use notify::{RecursiveMode, Watcher};
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

    try_join(watch(&config), serve(&config)).await?;
    Ok(())
}

async fn watch(config: &Config) -> anyhow::Result<()> {
    let mut watcher = notify::recommended_watcher(|res| match res {
        Ok(_) => todo!(),
        Err(_) => todo!(),
    })?;

    watcher.watch(config.path.as_ref(), RecursiveMode::NonRecursive)?;
    watcher.watch("content".as_ref(), RecursiveMode::Recursive)?;
    watcher.watch("theme".as_ref(), RecursiveMode::Recursive)?;

    tokio::task::spawn_blocking(|| {
        // TODO
    })
    .await?;

    Ok(())
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
