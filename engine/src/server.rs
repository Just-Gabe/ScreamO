//! Servidor local (127.0.0.1): entrega a UI e fala com ela por WebSocket.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::json;
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};

use crate::engine::{Action, Controller, Outcome};

/// UI compilada (ui/dist) embutida no executável.
#[derive(rust_embed::Embed)]
#[folder = "../ui/dist"]
struct EmbeddedUi;

#[derive(Clone)]
pub struct AppState {
    pub controller: Arc<Mutex<Controller>>,
    pub tx: broadcast::Sender<String>,
    pub port: u16,
    pub clients: Arc<ClientCount>,
}

/// Quantas janelas da UI estão conectadas (para fechar o motor junto com a janela).
#[derive(Default)]
pub struct ClientCount {
    pub connected: AtomicUsize,
    pub ever: AtomicBool,
}

/// `ui_dir` sobrescreve a UI embutida (útil para desenvolvimento).
pub fn router(state: AppState, ui_dir: Option<PathBuf>) -> Router {
    let app = Router::new().route("/ws", get(ws_handler));
    let app = match ui_dir {
        Some(dir) => app.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => app.fallback(embedded),
    };
    app.with_state(state)
}

async fn embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Rotas desconhecidas caem no index.html (SPA).
    let file = EmbeddedUi::get(if path.is_empty() { "index.html" } else { path }).or_else(|| EmbeddedUi::get("index.html"));
    match file {
        Some(f) => ([(header::CONTENT_TYPE, f.metadata.mimetype().to_string())], f.data).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "UI não embutida. Rode `npm run build` em ui/ e recompile o motor, ou use `npm run web`.",
        )
            .into_response(),
    }
}

/// Bloqueia sites de terceiros abertos no navegador de controlarem o motor
/// (WebSocket não tem proteção de CORS por padrão).
fn origin_allowed(headers: &HeaderMap, port: u16) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true; // clientes nativos (não-navegador) não mandam Origin
    };
    let allowed = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        // Servidor de desenvolvimento do Expo.
        "http://localhost:8081".to_string(),
        "http://127.0.0.1:8081".to_string(),
    ];
    allowed.iter().any(|a| a == origin)
}

async fn ws_handler(ws: WebSocketUpgrade, headers: HeaderMap, State(state): State<AppState>) -> Response {
    if !origin_allowed(&headers, state.port) {
        return (StatusCode::FORBIDDEN, "origem não permitida").into_response();
    }
    // Uploads de IR/NAM chegam em base64 numa única mensagem.
    let limit = crate::engine::files::MAX_FILE_BYTES * 2;
    ws.max_message_size(limit).max_frame_size(limit).on_upgrade(move |socket| client(socket, state))
}

async fn client(socket: WebSocket, state: AppState) {
    state.clients.connected.fetch_add(1, Ordering::SeqCst);
    state.clients.ever.store(true, Ordering::SeqCst);
    serve_client(socket, &state).await;
    state.clients.connected.fetch_sub(1, Ordering::SeqCst);
}

async fn serve_client(socket: WebSocket, state: &AppState) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.tx.subscribe();

    let initial = state.controller.lock().state_json().to_string();
    if sink.send(Message::Text(initial.into())).await.is_err() {
        return;
    }

    let (direct_tx, mut direct_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                m = rx.recv() => match m {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                },
                m = direct_rx.recv() => match m {
                    Some(m) => m,
                    None => break,
                },
            };
            if sink.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else { continue };
        let action: Action = match serde_json::from_str(&text) {
            Ok(a) => a,
            Err(e) => {
                let _ = direct_tx.send(json!({"type": "error", "message": format!("mensagem inválida: {e}")}).to_string());
                continue;
            }
        };
        let controller = state.controller.clone();
        // Ações de áudio abrem/fecham dispositivos (bloqueante).
        let result = tokio::task::spawn_blocking(move || {
            let mut c = controller.lock();
            c.apply(action).map(|o| (o, c.state_json()))
        })
        .await;
        match result {
            Ok(Ok((Outcome::State, s))) => {
                let _ = state.tx.send(s.to_string());
            }
            Ok(Ok((Outcome::Param { slot_id, index, value }, _))) => {
                let msg = json!({"type": "param", "slotId": slot_id, "index": index, "value": value});
                let _ = state.tx.send(msg.to_string());
            }
            Ok(Err(e)) => {
                let _ = direct_tx.send(json!({"type": "error", "message": format!("{e:#}")}).to_string());
            }
            Err(e) => eprintln!("[screamo] tarefa falhou: {e}"),
        }
    }
    writer.abort();
}

pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

pub fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}
