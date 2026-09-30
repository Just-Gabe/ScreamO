//! ScreamO — pedaleira de guitarra em tempo real para Windows.
//!
//! Uso: `screamo [--port 7777] [--no-browser]`

mod audio;
mod dsp;
mod engine;
mod server;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::broadcast;

use engine::Controller;
use server::AppState;

struct Args {
    port: u16,
    open_window: bool,
}

fn parse_args() -> Args {
    let mut args = Args { port: 7777, open_window: true };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => args.port = it.next().and_then(|p| p.parse().ok()).unwrap_or(7777),
            "--no-browser" => args.open_window = false,
            _ => eprintln!("argumento ignorado: {a}"),
        }
    }
    args
}

fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("ScreamO")
}

/// Procura a UI compilada: ao lado do .exe (release) ou em ui/dist (dev).
fn find_ui_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SCREAMO_UI_DIR") {
        return Some(PathBuf::from(p));
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let candidates = [
        exe_dir.join("ui"),
        exe_dir.join("../../../ui/dist"),
        PathBuf::from("ui/dist"),
        PathBuf::from("../ui/dist"),
    ];
    candidates.into_iter().find(|p| p.join("index.html").is_file())
}

/// Abre a UI como janela de app (sem barra de endereço) no Edge/Chrome.
fn open_window(url: &str) {
    let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();
    let pf = std::env::var("ProgramFiles").unwrap_or_default();
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let browsers = [
        format!("{pf86}\\Microsoft\\Edge\\Application\\msedge.exe"),
        format!("{pf}\\Microsoft\\Edge\\Application\\msedge.exe"),
        format!("{pf}\\Google\\Chrome\\Application\\chrome.exe"),
        format!("{local}\\Google\\Chrome\\Application\\chrome.exe"),
    ];
    for b in browsers.iter().filter(|b| std::path::Path::new(b).is_file()) {
        let ok = std::process::Command::new(b)
            .arg(format!("--app={url}"))
            .arg("--window-size=1400,900")
            .spawn()
            .is_ok();
        if ok {
            return;
        }
    }
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args();
    let url = server::url(args.port);

    let listener = match tokio::net::TcpListener::bind(server::addr(args.port)).await {
        Ok(l) => l,
        Err(_) => {
            // Já tem um ScreamO rodando: só abre a janela dele.
            println!("ScreamO já está rodando em {url}");
            if args.open_window {
                open_window(&url);
            }
            return Ok(());
        }
    };

    println!("ScreamO v{} — iniciando áudio...", env!("CARGO_PKG_VERSION"));
    let controller = tokio::task::spawn_blocking(|| {
        let mut c = Controller::new(data_dir());
        c.start_audio();
        c
    })
    .await?;
    let controller = Arc::new(Mutex::new(controller));
    {
        let s = controller.lock().state_json();
        match (&s["audio"]["input"], &s["audio"]["output"]) {
            (serde_json::Value::Object(i), serde_json::Value::Object(o)) => println!(
                "Entrada: {} ({})\nSaída:   {} ({})\nLatência estimada: {:.1} ms",
                i["deviceName"], i["mode"], o["deviceName"], o["mode"], s["audio"]["latencyMs"].as_f64().unwrap_or(0.0)
            ),
            _ => println!("Áudio parado: {}", s["audio"]["error"]),
        }
    }

    let (tx, _) = broadcast::channel::<String>(256);
    let state = AppState { controller: controller.clone(), tx: tx.clone(), port: args.port };

    // Medidores (~30 fps), limpeza e recuperação automática de dispositivo.
    {
        let (controller, tx) = (controller.clone(), tx.clone());
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(33));
            let mut n: u64 = 0;
            loop {
                interval.tick().await;
                n += 1;
                {
                    let mut c = controller.lock();
                    if c.tick() {
                        let _ = tx.send(c.state_json().to_string());
                    }
                    if tx.receiver_count() > 0 {
                        let _ = tx.send(c.meters_json().to_string());
                    }
                }
                if n % 90 == 0 {
                    let c = controller.clone();
                    let recovered = tokio::task::spawn_blocking(move || {
                        let mut c = c.lock();
                        c.try_recover().then(|| c.state_json())
                    })
                    .await
                    .ok()
                    .flatten();
                    if let Some(s) = recovered {
                        let _ = tx.send(s.to_string());
                    }
                }
            }
        });
    }

    let ui_dir = find_ui_dir();
    if ui_dir.is_none() {
        eprintln!("Aviso: UI compilada não encontrada (rode `npm run build` em ui/).");
    }
    let app = server::router(state, ui_dir);
    println!("UI: {url}  (Ctrl+C para sair)");
    if args.open_window {
        open_window(&url);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;

    println!("Encerrando...");
    tokio::task::spawn_blocking(move || { let mut c = controller.lock(); c.stop_audio(); c.tick(); }).await?;
    Ok(())
}
