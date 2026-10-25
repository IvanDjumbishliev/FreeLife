use std::path::Path;
use std::sync::{Arc, Mutex};

use freelife_core::FreeLifeMessage;
use freelife_transfer::node::Node;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    op: String,
    device_id: Option<String>,
    url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    devices: Option<Vec<Device>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn start(
    data_dir: &Path,
    node: Arc<Node>,
    app: AppHandle,
    feature_settings: Arc<Mutex<crate::FeatureSettings>>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use tokio::net::UnixListener;

        let socket_path = data_dir.join("handoff.sock");
        if let Ok(metadata) = std::fs::symlink_metadata(&socket_path) {
            if !metadata.file_type().is_socket() {
                return Err(
                    format!("refusing to replace non-socket {}", socket_path.display()).into(),
                );
            }
            match std::os::unix::net::UnixStream::connect(&socket_path) {
                Ok(_) => return Err("browser handoff IPC is already running".into()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) =>
                {
                    std::fs::remove_file(&socket_path)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        let listener = tauri::async_runtime::block_on(async { UnixListener::bind(&socket_path) })?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        tauri::async_runtime::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let node = node.clone();
                        let app = app.clone();
                        let feature_settings = feature_settings.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                serve(stream, node, app, feature_settings).await
                            {
                                tracing::warn!(%error, "browser handoff IPC request failed");
                            }
                        });
                    }
                    Err(error) => tracing::error!(%error, "browser handoff IPC accept failed"),
                }
            }
        });
    }

    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;

        let _ = data_dir;
        tauri::async_runtime::spawn(async move {
            loop {
                let server = match ServerOptions::new()
                    .first_pipe_instance(false)
                    .create(r"\\.\pipe\freelife-handoff")
                {
                    Ok(server) => server,
                    Err(error) => {
                        tracing::error!(%error, "could not create browser handoff pipe");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }
                };
                if let Err(error) = server.connect().await {
                    tracing::warn!(%error, "browser handoff pipe connection failed");
                    continue;
                }
                let node = node.clone();
                let app = app.clone();
                let feature_settings = feature_settings.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = serve(server, node, app, feature_settings).await {
                        tracing::warn!(%error, "browser handoff IPC request failed");
                    }
                });
            }
        });
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (data_dir, node, app, feature_settings);
        return Err("browser handoff IPC is not supported on this platform".into());
    }
    Ok(())
}

#[cfg(unix)]
async fn serve<S>(
    stream: S,
    node: Arc<Node>,
    app: AppHandle,
    feature_settings: Arc<Mutex<crate::FeatureSettings>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    serve_stream(stream, node, app, feature_settings).await
}

#[cfg(windows)]
async fn serve<S>(
    stream: S,
    node: Arc<Node>,
    app: AppHandle,
    feature_settings: Arc<Mutex<crate::FeatureSettings>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    serve_stream(stream, node, app, feature_settings).await
}

async fn serve_stream<S>(
    stream: S,
    node: Arc<Node>,
    app: AppHandle,
    feature_settings: Arc<Mutex<crate::FeatureSettings>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut stream = stream;
    let mut line = Vec::new();
    let mut chunk = [0; 1024];
    let mut oversized = false;
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let end = chunk[..read]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(read);
        if line.len() + end > MAX_REQUEST_BYTES {
            oversized = true;
            break;
        }
        line.extend_from_slice(&chunk[..end]);
        if chunk[end - 1] == b'\n' {
            break;
        }
    }
    let response = if oversized {
        Response::error("request exceeds the size limit")
    } else if line.is_empty() {
        Response::error("empty request")
    } else {
        match serde_json::from_slice(&line) {
            Ok(request) => handle_request(request, node, app, feature_settings).await,
            Err(error) => Response::error(&format!("invalid request: {error}")),
        }
    };
    stream.write_all(&serde_json::to_vec(&response)?).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn handle_request(
    request: Request,
    node: Arc<Node>,
    app: AppHandle,
    feature_settings: Arc<Mutex<crate::FeatureSettings>>,
) -> Response {
    match feature_settings.lock() {
        Ok(settings) if settings.handoff_enabled => {}
        Ok(_) => return Response::error("Handoff is disabled in FreeLife settings"),
        Err(error) => {
            tracing::error!(%error, "could not read feature settings");
            return Response::error("could not read FreeLife settings");
        }
    }
    match request.op.as_str() {
        "getDevices" => Response {
            ok: true,
            devices: Some(
                node.paired_devices()
                    .into_iter()
                    .map(|peer| Device {
                        id: peer.id.to_string(),
                        name: peer.name,
                    })
                    .collect(),
            ),
            error: None,
        },
        "handoff" => {
            let Some(device_id) = request.device_id else {
                return Response::error("deviceId is required");
            };
            let Some(url) = request.url else {
                return Response::error("url is required");
            };
            let Ok(device_id) = device_id.parse() else {
                return Response::error("invalid deviceId");
            };
            if !node.is_paired(device_id) {
                return Response::error("device is not paired");
            }
            if !valid_handoff_url(&url) {
                return Response::error("only http and https URLs can be handed off");
            }
            match node.connection_to(device_id).await {
                Ok(connection) => match connection
                    .send(FreeLifeMessage::HandoffUrl { url: url.clone() })
                    .await
                {
                    Ok(()) => {
                        crate::events::record_local_handoff(&app, &node, url);
                        Response::success()
                    }
                    Err(error) => Response::error(&error.to_string()),
                },
                Err(error) => Response::error(&error.to_string()),
            }
        }
        _ => Response::error("unknown operation"),
    }
}

fn valid_handoff_url(value: &str) -> bool {
    url::Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .unwrap_or(false)
}

impl Response {
    fn success() -> Self {
        Self {
            ok: true,
            devices: None,
            error: None,
        }
    }

    fn error(message: &str) -> Self {
        Self {
            ok: false,
            devices: None,
            error: Some(message.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::valid_handoff_url;

    #[test]
    fn accepts_only_web_urls() {
        assert!(valid_handoff_url("https://example.com/path"));
        assert!(valid_handoff_url("http://localhost:8080/"));
        assert!(!valid_handoff_url("javascript:alert(1)"));
        assert!(!valid_handoff_url("file:///etc/passwd"));
        assert!(!valid_handoff_url("https:"));
    }
}
