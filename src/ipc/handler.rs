use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Notify;
use tracing::{error, info};

use crate::daemon::DaemonState;
use crate::ipc::protocol::{Request, Response};

pub async fn serve(
    socket_path: &Path,
    state: Arc<DaemonState>,
    activity: Arc<Notify>,
) -> std::io::Result<()> {
    // Remove stale socket
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    info!("IPC listening on {}", socket_path.display());

    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);
        let activity = Arc::clone(&activity);

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut lines = BufReader::new(reader).lines();

            while let Ok(Some(line)) = lines.next_line().await {
                // Signal activity for idle timeout
                activity.notify_one();

                let response = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => handle_request(req, &state).await,
                    Err(e) => Response::error(format!("invalid request: {e}")),
                };

                let mut resp_bytes = serde_json::to_vec(&response).unwrap();
                resp_bytes.push(b'\n');

                if let Err(e) = writer.write_all(&resp_bytes).await {
                    error!("write error: {e}");
                    break;
                }
            }
        });
    }
}

async fn handle_request(req: Request, state: &DaemonState) -> Response {
    match req {
        Request::Ingest(params) => state.handle_ingest(params).await,
        Request::GetContext(params) => state.handle_get_context(params).await,
        Request::GetStatus => state.handle_get_status(),
        Request::EndSession(params) => state.handle_end_session(params),
        Request::Search(params) => state.handle_search(params),
    }
}
