#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod logs;
mod paths;
mod selftest;
mod server;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use handwriter_core::i18n::tr;
use tauri::Manager;

use crate::server::Activity;

const PING_IDLE_S: f64 = 15.0;
const STARTUP_GRACE_S: f64 = 30.0;

struct Args {
    browser: bool,
    port: Option<u16>,
    no_open: bool,
    selftest: bool,
    report: Option<PathBuf>,
}

fn usage() -> String {
    tr("HandWriter: почерк и чертежи карандашом\n\n\
        --browser      режим разработки: открыть обычную вкладку, не завершаться после закрытия окна\n\
        --port N       порт (по умолчанию свободный; с --browser 8765)\n\
        --no-open      не открывать окно\n\
        --selftest     самопроверка и выход\n\
        --report PATH  куда записать отчёт самопроверки")
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { browser: false, port: None, no_open: false, selftest: false, report: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--browser" => a.browser = true,
            "--no-open" => a.no_open = true,
            "--selftest" => a.selftest = true,
            "--port" => a.port = Some(it.next().and_then(|v| v.parse().ok()).ok_or_else(usage)?),
            "--report" => a.report = Some(it.next().ok_or_else(usage)?.into()),
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("{}: {other}\n\n{}", tr("неизвестный параметр"), usage())),
        }
    }
    Ok(a)
}

fn bind(port: Option<u16>) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
}

fn spawn_server(listener: TcpListener, activity: Arc<Activity>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("server".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
            rt.block_on(async move {
                listener.set_nonblocking(true).expect("nonblocking");
                let l = tokio::net::TcpListener::from_std(listener).expect("listener");
                if let Err(e) = axum::serve(l, server::router(activity)).await {
                    logs::error("handwriter.server", &format!("Сервер остановлен с ошибкой: {e}"));
                }
            });
        })
        .expect("server thread")
}

fn open_browser(url: &str) {
    let cmd: (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else {
        ("xdg-open", vec![url])
    };
    if let Err(e) = std::process::Command::new(cmd.0).args(&cmd.1).spawn() {
        logs::warn("handwriter.launcher", &format!("Не удалось открыть браузер: {e}"));
    }
}

fn should_stop(now: f64, last_ping: Option<f64>) -> bool {
    match last_ping {
        None => now > STARTUP_GRACE_S,
        Some(p) => now - p > PING_IDLE_S && now > STARTUP_GRACE_S,
    }
}

fn run_server_only(args: &Args) -> i32 {
    let port = args.port.or(if args.browser { Some(8765) } else { None });
    let listener = match bind(port) {
        Ok(l) => l,
        Err(e) => return fatal(&format!("Программа не запустилась: {e}")),
    };
    let url = format!("http://127.0.0.1:{}/", listener.local_addr().map(|a| a.port()).unwrap_or(0));
    let activity = Arc::new(Activity::new());
    let handle = spawn_server(listener, activity.clone());
    logs::info("handwriter.launcher", &format!("Сервер {url} готов"));
    println!("HandWriter: {url}  ({})", tr("Ctrl+C для выхода"));
    if !args.no_open {
        open_browser(&url);
    }
    if args.browser {
        let _ = handle.join();
        return 0;
    }
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if should_stop(activity.started.elapsed().as_secs_f64(), activity.last_ping()) {
            logs::info("handwriter.launcher", &format!("Окно закрыто (нет пингов {PING_IDLE_S} с), завершаю сервер"));
            return 0;
        }
    }
}

fn run_window(args: &Args) -> i32 {
    let port = args.port;
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(move |app| {
            let listener = bind(port)?;
            let url = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
            spawn_server(listener, Arc::new(Activity::new()));
            logs::info("handwriter.launcher", &format!("Сервер {url} готов, открываю окно"));
            let app_url = format!("{url}?app=1");
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(app_url.parse()?))
                .title("HandWriter")
                .inner_size(1400.0, 900.0)
                .min_inner_size(800.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!());
    match result {
        Ok(()) => 0,
        Err(e) => fatal(&format!("Программа не запустилась: {e}")),
    }
}

fn fatal(text: &str) -> i32 {
    let msg = tr(&format!("{text}\n\nПодробности в журнале:\n{}", paths::log_path().display()));
    logs::error("handwriter.launcher", text);
    eprintln!("{msg}");
    1
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            println!("{msg}");
            std::process::exit(2);
        }
    };
    logs::setup(true);
    if args.selftest {
        std::process::exit(selftest::run(args.report.clone()));
    }
    if let Err(e) = assets::install_fonts() {
        std::process::exit(fatal(&format!("Не удалось подготовить встроенные шрифты: {e}")));
    }
    let code = if args.browser || args.no_open { run_server_only(&args) } else { run_window(&args) };
    std::process::exit(code);
}
