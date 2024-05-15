use std::{fs::OpenOptions, io::Write, path::PathBuf};

use json::JsonValue;
use krossbar_hub_lib::{args::Args, hub::Hub};
use log::LevelFilter;
use rstest::fixture;
use tempdir::TempDir;
use tokio_util::sync::CancellationToken;

pub struct Fixture {
    socket_path: PathBuf,
    // Need this to keep temp dir from deletion
    _socket_dir: TempDir,
    service_files_dir: TempDir,
    cancel_token: CancellationToken,
}

impl Fixture {
    pub async fn new(log_level: LevelFilter) -> Self {
        let _ = pretty_env_logger::formatted_builder()
            .filter_level(log_level)
            .try_init();

        let socket_dir =
            TempDir::new("krossbar_hub_socket_dir").expect("Failed to create socket tempdir");

        let socket_path: PathBuf = socket_dir.path().join("krossbar_hub.socket");

        let service_files_dir = TempDir::new("krossbar_test_non_existing_connection")
            .expect("Failed to create tempdir");

        let this = Self {
            socket_path,
            _socket_dir: socket_dir,
            service_files_dir,
            cancel_token: CancellationToken::new(),
        };

        this.start_hub(log_level).await;
        this
    }

    async fn start_hub(&self, log_level: LevelFilter) {
        let args = Args {
            log_level,
            additional_service_dirs: vec![self.service_files_dir.path().into()],
            socket_path: self.socket_path.clone(),
        };

        let token = self.cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = Hub::new(args).run() => {}
                _ = token.cancelled() => {}
            }
        });
    }

    pub fn write_service_file(&self, service_name: &str, content: JsonValue) {
        let service_file_path = self
            .service_files_dir
            .path()
            .join(format!("{}.service", service_name));

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(service_file_path.as_path())
            .expect("Failed to create service file");

        file.write_all(json::stringify(content).as_bytes())
            .expect("Failed to write service file content");
        file.flush().expect("Failed to flush service file");
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel()
    }

    pub fn hub_socket_path(&self) -> &PathBuf {
        &self.socket_path
    }
}

#[fixture]
pub async fn make_fixture() -> Fixture {
    Fixture::new(LevelFilter::Debug).await
}

#[fixture]
pub async fn make_warn_fixture() -> Fixture {
    Fixture::new(LevelFilter::Warn).await
}
