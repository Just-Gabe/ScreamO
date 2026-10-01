//! ScreamO — pedaleira de guitarra em tempo real para Windows.
//!
//! Uso: `screamo [--port 7777] [--no-browser] [--keep-running]`
//!
//! No build release não há janela de console; ao fechar a janela da UI o
//! motor encerra sozinho (a menos que `--keep-running` seja usado).

// Sem console no release (é um app de janela). Em debug o console fica.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod dsp;
mod engine;
mod server;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::broadcast;

use engine::Controller;
use server::{AppState, ClientCount};

/// Tempo sem nenhuma janela conectada antes de encerrar (cobre recarregar a página).
const EXIT_GRACE: Duration = Duration::from_secs(5);

struct Args {
    port: u16,
    open_window: bool,
    keep_running: bool,
}

fn parse_args() -> Args {
    let mut args = Args { port: 7777, open_window: true, keep_running: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => args.port = it.next().and_then(|p| p.parse().ok()).unwrap_or(7777),
            "--no-browser" => args.open_window = false,
            "--keep-running" => args.keep_running = true,
            _ => eprintln!("argumento ignorado: {a}"),
        }
    }
    args
}

/// `SCREAMO_DATA_DIR` permite uma pasta isolada (testes, perfis).
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SCREAMO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("ScreamO")
}

/// Quando aberto por um terminal, reaproveita o console dele para os logs
/// (o executável release não cria console próprio).
fn attach_parent_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
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
    attach_parent_console();
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
    let clients = Arc::new(ClientCount::default());
    let state = AppState { controller: controller.clone(), tx: tx.clone(), port: args.port, clients: clients.clone() };
    let shutdown = Arc::new(tokio::sync::Notify::new());

    // Encerra junto com a janela: depois que alguma UI conectou, se ficar
    // EXIT_GRACE sem nenhuma, o motor sai.
    if !args.keep_running {
        let (clients, shutdown) = (clients.clone(), shutdown.clone());
        tokio::spawn(async move {
            let mut empty_since: Option<Instant> = None;
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let empty = clients.ever.load(Ordering::SeqCst) && clients.connected.load(Ordering::SeqCst) == 0;
                match (empty, empty_since) {
                    (true, None) => empty_since = Some(Instant::now()),
                    (true, Some(t)) if t.elapsed() >= EXIT_GRACE => {
                        println!("Janela fechada.");
                        shutdown.notify_one();
                        return;
                    }
                    (false, _) => empty_since = None,
                    _ => {}
                }
            }
        });
    }

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

    // SCREAMO_UI_DIR serve a UI do disco em vez da embutida (desenvolvimento).
    let ui_dir = std::env::var_os("SCREAMO_UI_DIR").map(PathBuf::from);
    let app = server::router(state, ui_dir);
    println!("UI: {url}  (feche a janela ou Ctrl+C para sair)");
    if args.open_window {
        open_window(&url);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = shutdown.notified() => {}
            }
        })
        .await?;

    println!("Encerrando...");
    tokio::task::spawn_blocking(move || {
        let mut c = controller.lock();
        c.stop_audio();
        c.tick();
    })
    .await?;
    // O servidor pode ter tarefas presas em WebSockets abertos; sai de vez.
    std::process::exit(0)
}
