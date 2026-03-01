use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use core::io::output::OutputReceiver;
use core::types::SensoryEvent;
use llm::provider::LlmProvider;
use rustyline::error::ReadlineError;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const DB_CONNECT_TIMEOUT_SECS: u64 = 3;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut startup_notice: Option<String> = None;
    let pool = if let Ok(url) = std::env::var("DATABASE_URL") {
        let mut fallback = |reason: String| {
            startup_notice = Some(format!(
                "提示：{reason}，已自动降级为临时模式（ephemeral）。本次会话数据不会持久化。"
            ));
        };
        match tokio::time::timeout(
            Duration::from_secs(DB_CONNECT_TIMEOUT_SECS),
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(8)
                .connect(&url),
        )
        .await
        {
            Ok(Ok(pool)) => match sqlx::migrate!("../../migrations").run(&pool).await {
                Ok(()) => Some(pool),
                Err(_) => { fallback("数据库迁移失败".into()); None }
            },
            Ok(Err(_)) => { fallback("无法连接 DATABASE_URL".into()); None }
            Err(_) => { fallback(format!("连接数据库超时（{DB_CONNECT_TIMEOUT_SECS}s）")); None }
        }
    } else {
        None
    };

    let cfg = if let Some(ref pool) = pool {
        core::config::IrisCfg::load(pool).await?
    } else {
        core::config::IrisCfg::default()
    };
    let cfg = Arc::new(cfg);

    let llm: Option<Arc<dyn LlmProvider>> = llm::http::from_env().map(|p| Arc::new(p) as _);
    let lite_llm: Option<Arc<dyn LlmProvider>> =
        llm::http::lite_from_env().map(|p| Arc::new(p) as _);

    let (mut runtime, event_tx, output_rx) = core::runtime::Runtime::new(cfg, pool, llm, lite_llm);
    let token = runtime.token();
    spawn_sigint_canceler(token.clone());

    let repl_token = token.clone();
    let runtime_fut = runtime.run();
    let repl_fut = run_repl(event_tx, output_rx, repl_token, startup_notice);
    tokio::pin!(runtime_fut);
    tokio::pin!(repl_fut);

    tokio::select! {
        _ = &mut runtime_fut => {
            token.cancel();
            (&mut repl_fut).await
        }
        result = &mut repl_fut => {
            token.cancel();
            (&mut runtime_fut).await;
            result
        }
    }
}

async fn run_repl(
    event_tx: mpsc::Sender<SensoryEvent>,
    mut output_rx: OutputReceiver,
    token: CancellationToken,
    startup_notice: Option<String>,
) -> anyhow::Result<()> {
    const SPINNER: [&str; 4] = ["-", "\\", "|", "/"];

    // Clear screen so cargo build output is not visible.
    print!("\x1b[2J\x1b[3J\x1b[H");
    io::stdout().flush()?;

    if let Some(notice) = startup_notice {
        println!("{notice}");
    }
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<InputEvent>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    spawn_input_thread(line_tx, ready_rx);
    request_next_prompt(&ready_tx);

    let mut waiting_for_reply = false;
    let mut spinner_idx: usize = 0;
    let mut spinner_interval = tokio::time::interval(Duration::from_millis(100));
    spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                break;
            }
            _ = spinner_interval.tick(), if waiting_for_reply => {
                spinner_idx = (spinner_idx + 1) % SPINNER.len();
                draw_thinking_frame(SPINNER[spinner_idx])?;
            }
            line = line_rx.recv() => {
                let Some(line) = line else {
                    break;
                };
                match line {
                    InputEvent::Line(line) => {
                        let text = line.trim();
                        if text.is_empty() {
                            request_next_prompt(&ready_tx);
                            continue;
                        }
                        if matches!(text, "/q" | "/exit" | "/quit") {
                            break;
                        }
                        if core::io::input::submit_text(&event_tx, text.to_owned()).await.is_err() {
                            break;
                        }
                        if !waiting_for_reply {
                            spinner_idx = 0;
                            draw_thinking_frame(SPINNER[spinner_idx])?;
                            waiting_for_reply = true;
                        }
                    }
                    InputEvent::Interrupted => {
                        token.cancel();
                        break;
                    }
                    InputEvent::Eof => break,
                    InputEvent::Error(err) => {
                        eprintln!("input error: {err}");
                        break;
                    }
                }
            }
            msg = output_rx.recv() => {
                let Some(msg) = msg else {
                    break;
                };
                if msg.is_streaming {
                    if waiting_for_reply {
                        waiting_for_reply = false;
                        clear_current_line()?;
                    }
                    print!("{}", msg.content);
                    io::stdout().flush()?;
                } else {
                    if waiting_for_reply {
                        waiting_for_reply = false;
                        clear_current_line()?;
                    }
                    println!("{}", msg.content);
                    request_next_prompt(&ready_tx);
                }
            }
        }
    }
    drop(ready_tx);

    if waiting_for_reply {
        clear_current_line()?;
    }
    println!();
    Ok(())
}

fn draw_thinking_frame(frame: &str) -> anyhow::Result<()> {
    print!("\rthinking... {frame}");
    io::stdout().flush()?;
    Ok(())
}

fn clear_current_line() -> anyhow::Result<()> {
    print!("\r\x1b[2K");
    io::stdout().flush()?;
    Ok(())
}

fn request_next_prompt(ready_tx: &std::sync::mpsc::Sender<()>) {
    let _ = ready_tx.send(());
}

fn spawn_input_thread(
    line_tx: mpsc::UnboundedSender<InputEvent>,
    ready_rx: std::sync::mpsc::Receiver<()>,
) {
    std::thread::spawn(move || {
        let mut editor = match rustyline::DefaultEditor::new() {
            Ok(editor) => editor,
            Err(e) => {
                let _ = line_tx.send(InputEvent::Error(e.to_string()));
                return;
            }
        };

        while ready_rx.recv().is_ok() {
            match editor.readline("You> ") {
                Ok(line) => {
                    if line_tx.send(InputEvent::Line(line)).is_err() {
                        break;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    let _ = line_tx.send(InputEvent::Interrupted);
                    break;
                }
                Err(ReadlineError::Eof) => {
                    let _ = line_tx.send(InputEvent::Eof);
                    break;
                }
                Err(e) => {
                    let _ = line_tx.send(InputEvent::Error(e.to_string()));
                    break;
                }
            }
        }
    });
}

enum InputEvent {
    Line(String),
    Interrupted,
    Eof,
    Error(String),
}

fn spawn_sigint_canceler(token: CancellationToken) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut sigint) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            {
                let _ = sigint.recv().await;
                token.cancel();
            }
        }
        #[cfg(not(unix))]
        {
            if tokio::signal::ctrl_c().await.is_ok() {
                token.cancel();
            }
        }
    });
}
