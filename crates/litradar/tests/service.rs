//! Real-process lifecycle tests for the unified LitRadar service.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::Duration;

use litradar_auth::{AuthService, ACCESS_TOKEN_DEFAULT_TTL};
use serde_json::Value;
use tempfile::tempdir;

const SERVICE_START_ATTEMPTS: usize = 200;
const SERVICE_START_RETRY_DELAY: Duration = Duration::from_millis(25);
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

#[test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support child processes or TCP listeners"
)]
fn unified_service_serves_frontend_openapi_and_authenticated_api_then_cleans_up() {
    let temp_dir = tempdir().expect("temporary service root should be created");
    let root_path = temp_dir.path().to_path_buf();
    let storage_config = litradar_storage::StorageConfig::from_project_root(&root_path);
    let secret_key_file = root_path.join("secret.key");
    fs::write(&secret_key_file, [31_u8; 32]).expect("secret key should write");
    fs::create_dir_all(root_path.join("web")).expect("web root should be created");
    fs::write(
        root_path.join("web").join("index.html"),
        "<!doctype html><main>service-process-marker</main>",
    )
    .expect("frontend marker should write");
    litradar_storage::migrate_storage(&storage_config).expect("storage should migrate");
    let auth_service = AuthService::new(storage_config.auth_db_path());
    let administrator = auth_service
        .bootstrap_admin("service_admin", "fixture-password")
        .expect("service administrator should bootstrap");
    let access_token = auth_service
        .create_access_token(administrator.id, "service-test", ACCESS_TOKEN_DEFAULT_TTL)
        .expect("service access token should be created")
        .token;
    litradar_storage::upsert_runtime_settings(
        storage_config.auth_db_path(),
        &litradar_storage::SecretCodec::from_key([31_u8; 32]),
        &HashMap::from([
            ("log_format".to_string(), Some("compact".to_string())),
            ("log_filter".to_string(), Some("off".to_string())),
        ]),
        &HashMap::new(),
    )
    .expect("service logging settings should persist");
    let port = reserve_loopback_port();
    let mut service = ServiceChild::spawn(&root_path, port);

    let ready = wait_for_ready(&mut service, port);
    let root = http_get(port, "/", None).expect("frontend root should respond");
    let openapi = http_get(port, "/openapi.json", None).expect("OpenAPI should respond");
    let authenticated = http_get(
        port,
        "/api/auth/me",
        Some(&format!("Bearer {access_token}")),
    )
    .expect("authenticated API should respond");

    assert_eq!(ready.status, 200);
    assert_eq!(ready.json()["status"], "ok");
    assert_eq!(root.status, 200);
    assert!(root.body.contains("service-process-marker"));
    assert_eq!(openapi.status, 200);
    assert!(openapi.json()["paths"]["/api/auth/me"].is_object());
    assert_eq!(authenticated.status, 200);
    assert_eq!(authenticated.json()["username"], "service_admin");
    assert_eq!(authenticated.json()["is_admin"], true);

    let output = service.terminate();
    assert!(wait_for_port_release(port));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.is_empty());
    assert!(!stderr.contains(&access_token));
    drop(output);
    drop(temp_dir);
    assert!(!root_path.exists());
}

struct ServiceChild {
    child: Option<Child>,
}

impl ServiceChild {
    fn spawn(project_root: &Path, port: u16) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_litradar"))
            .current_dir(project_root)
            .args([
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--project-root",
                ".",
                "--secret-key-file",
                "secret.key",
                "--scheduler-interval-seconds",
                "3600",
            ])
            .env_remove("LITRADAR_BUNDLED_META_DIR")
            .env_remove("RUST_LOG")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("service process should start");
        Self { child: Some(child) }
    }

    fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        self.child
            .as_mut()
            .expect("service child should exist")
            .try_wait()
            .expect("service status should be readable")
    }

    fn terminate(mut self) -> Output {
        let mut child = self.child.take().expect("service child should exist");
        if child
            .try_wait()
            .expect("service status should be readable")
            .is_none()
        {
            child.kill().expect("service process should terminate");
        }
        child
            .wait_with_output()
            .expect("service process output should be collected")
    }
}

impl Drop for ServiceChild {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

impl HttpResponse {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("HTTP response body should be JSON")
    }
}

fn reserve_loopback_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("ephemeral loopback port should bind")
        .local_addr()
        .expect("ephemeral loopback address should resolve")
        .port()
}

fn wait_for_ready(service: &mut ServiceChild, port: u16) -> HttpResponse {
    for _ in 0..SERVICE_START_ATTEMPTS {
        if let Some(status) = service.try_wait() {
            panic!("service exited before readiness with {status}");
        }
        if let Ok(response) = http_get(port, "/health/ready", None) {
            if response.status == 200 {
                return response;
            }
        }
        thread::sleep(SERVICE_START_RETRY_DELAY);
    }
    panic!(
        "service did not become ready after {} ms",
        SERVICE_START_ATTEMPTS * SERVICE_START_RETRY_DELAY.as_millis() as usize
    );
}

fn wait_for_port_release(port: u16) -> bool {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    for _ in 0..20 {
        if TcpStream::connect_timeout(&address, Duration::from_millis(25)).is_err() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn http_get(port: u16, path: &str, authorization: Option<&str>) -> std::io::Result<HttpResponse> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = TcpStream::connect_timeout(&address, HTTP_TIMEOUT)?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT))?;
    stream.set_write_timeout(Some(HTTP_TIMEOUT))?;
    let authorization = authorization
        .map(|value| format!("Authorization: {value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{authorization}Connection: close\r\n\r\n"
    )?;
    stream.flush()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    let response = String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let (headers, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "HTTP headers missing")
    })?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "HTTP status missing")
        })?;
    Ok(HttpResponse {
        status,
        body: body.to_string(),
    })
}
