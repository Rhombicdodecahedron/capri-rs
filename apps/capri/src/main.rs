//! Composition root. The only place that knows every adapter: it reads the config, builds the adapters,
//! plugs them into the application services and starts a driving adapter (the desk or the terminal).

mod brain;
mod desk;
mod preview;
mod voice;

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use capri_application::ports::{MemoryStore, SkinRepository, UiDriver};
use capri_application::services::{
    ChatEvent, ChatService, ChatSettings, Hands, RememberTool, SpotifyRemote, ToolRunner,
};
use capri_config::Config;
use capri_memory_sqlite::SqliteMemory;
use capri_settings_egui::{SettingsInput, SkinChoice};
use capri_skin_fs::FsSkins;
use clap::{Parser, Subcommand};
use tokio::io::AsyncBufReadExt;

#[derive(Parser, Debug)]
#[command(name = "capri", version, about = "A cyberpunk AI companion that lives on your desktop.")]
struct Cli {
    /// Config file (default: the user config folder's capri.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Put her on the desktop (the default).
    Desk,
    /// Talk to her in the terminal: same brain and memory, no window.
    Chat,
    /// Open her settings panel (right-click her on the desk does the same).
    Settings {
        /// Test the brain connection as soon as the panel opens.
        #[arg(long)]
        test: bool,
    },
    /// Test the configured brain: list models, time a short reply.
    Brain,
    /// List installed skins.
    Skins,
    /// Print where the config, data and skins live.
    Paths,
    /// Render a contact sheet of a skin's poses to a PNG, to check new art without starting her.
    Preview {
        /// Skin id (folder name under assets/skins).
        #[arg(default_value = "demo")]
        skin: String,
        #[arg(long, default_value = "preview.png")]
        out: PathBuf,
    },
}

struct Paths {
    config: PathBuf,
    data: PathBuf,
    skin_roots: Vec<PathBuf>,
}

fn paths(cli_config: Option<PathBuf>) -> Paths {
    let dirs = directories::ProjectDirs::from("ch", "capri", "capri");
    let config = cli_config.unwrap_or_else(|| {
        dirs.as_ref().map_or_else(|| PathBuf::from("capri.toml"), |d| d.config_dir().join("capri.toml"))
    });
    let data = dirs.as_ref().map_or_else(|| PathBuf::from("data"), |d| d.data_dir().to_path_buf());
    // bundled skins next to the binary or in the repo, then the user's own folder (wins on equal names)
    let mut skin_roots = vec![PathBuf::from("assets/skins")];
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
        skin_roots.push(exe_dir.join("assets/skins"));
    }
    skin_roots.push(data.join("skins"));
    Paths { config, data, skin_roots }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cli = Cli::parse();
    let p = paths(cli.config);
    let cfg = Config::load_or_create(&p.config)?;
    let skins = Arc::new(FsSkins::new(p.skin_roots.clone()));

    match cli.command.unwrap_or(Command::Desk) {
        Command::Paths => {
            println!("config: {}", p.config.display());
            println!("data:   {}", p.data.display());
            for r in &p.skin_roots {
                println!("skins:  {}", r.display());
            }
            Ok(())
        }
        Command::Skins => {
            for s in skins.list() {
                println!("{:<16} {}{}", s.id, s.name, s.credits.map(|c| format!("  ({c})")).unwrap_or_default());
            }
            Ok(())
        }
        Command::Preview { skin, out } => {
            preview::render(skins.as_ref(), &skin, cfg.desk.size, &out)?;
            println!("wrote {}", out.display());
            Ok(())
        }
        Command::Chat => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(terminal_chat(chat_service(&cfg, &p)?.chat))
        }
        Command::Desk => {
            let rt = tokio::runtime::Runtime::new()?;
            let powers = chat_service(&cfg, &p)?;
            rt.block_on(powers.chat.resume());
            desk::run(&cfg, p.config.clone(), p.data.clone(), skins, powers, &rt)
        }
        Command::Settings { test } => {
            let rt = tokio::runtime::Runtime::new()?;
            let input = SettingsInput {
                config_path: p.config.clone(),
                skins: skins.list().into_iter().map(|s| SkinChoice { id: s.id, name: s.name }).collect(),
                tester: Arc::new(brain::Tester::new(p.data.clone())),
                runtime: rt.handle().clone(),
                system_locale: sys_locale::get_locale(),
                test_on_open: test,
            };
            capri_settings_egui::run(input).map_err(|e| anyhow::anyhow!(e))
        }
        Command::Brain => {
            let rt = tokio::runtime::Runtime::new()?;
            let b = brain::from_config(&cfg)?;
            println!("testing {} ...", b.model().name());
            match rt.block_on(brain::test(&b)) {
                Ok(r) => {
                    if !r.models.is_empty() {
                        println!("models:      {}", r.models.join(", "));
                    }
                    println!("model:       {}", r.model);
                    println!("first word:  {} ms", r.first_word_ms);
                    println!("total:       {} ms", r.total_ms);
                    println!("reply:       {}", r.reply.trim());
                    Ok(())
                }
                Err(e) => {
                    if !e.models.is_empty() {
                        println!("models on the server: {}", e.models.join(", "));
                    }
                    anyhow::bail!("brain offline: {}", e.message)
                }
            }
        }
    }
}

/// Calls that don't come from a chat have no one to ask: anything that needs a yes is declined.
/// (Everything she does for a typed or spoken line asks in that chat instead.)
#[derive(Debug)]
struct NoOneToAsk;

#[async_trait::async_trait]
impl capri_application::ports::ApprovalPrompt for NoOneToAsk {
    async fn ask(&self, request: &capri_application::ports::ActionRequest) -> bool {
        tracing::info!(tool = %request.tool, summary = %request.summary, "declined: no chat to ask in");
        false
    }
}

/// The chat and the parts of her powers the settings can change live.
#[derive(Debug)]
pub struct Powers {
    pub chat: Arc<ChatService>,
    pub tools: Arc<ToolRunner>,
    /// Her hands on other apps, when this system has them.
    pub hands: Option<Arc<Hands>>,
}

fn chat_service(cfg: &Config, p: &Paths) -> anyhow::Result<Powers> {
    let model = brain::from_config(cfg)?.model();
    let db = cfg.memory.path.clone().unwrap_or_else(|| p.data.join("memory.sqlite"));
    let memory: Arc<dyn MemoryStore> =
        Arc::new(SqliteMemory::open(&db).with_context(|| format!("opening {}", db.display()))?);
    let lang = cfg.ui.language.resolve(sys_locale::get_locale().as_deref());
    let settings = ChatSettings { persona: cfg.persona.persona(), lang, ..ChatSettings::default() };
    let mut tools = ToolRunner::new(cfg.safety.approval, Arc::new(NoOneToAsk))
        .with(Arc::new(capri_media_os::MediaKeys))
        .with(Arc::new(RememberTool::new(memory.clone())))
        .with_all(capri_system_os::tools());
    let mut hands = None;
    if capri_ui_windows::AVAILABLE {
        let ui: Arc<dyn UiDriver> = Arc::new(capri_ui_windows::Automation);
        tools = tools.with(Arc::new(SpotifyRemote::new(ui.clone(), Arc::new(capri_system_os::SystemOpener))));
        let h = Hands::new(ui, cfg.safety.never_touch.clone());
        h.configure(cfg.safety.app_control, cfg.safety.never_touch.clone());
        tools = tools.with_all(h.tools());
        hands = Some(h);
    }
    let tools = Arc::new(tools);
    tracing::info!(tools = ?tools.specs().iter().map(|t| t.name.clone()).collect::<Vec<_>>(), "powers ready");
    let chat = Arc::new(ChatService::new(model, memory, settings).with_tools(tools.clone()));
    Ok(Powers { chat, tools, hands })
}

async fn terminal_chat(chat: Arc<ChatService>) -> anyhow::Result<()> {
    chat.resume().await;
    println!("Connected to {}. Type to talk, /quit to leave.", chat.model_name());
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("you > ");
        std::io::stdout().flush()?;
        let Some(line) = lines.next_line().await? else { break };
        if matches!(line.trim(), "/quit" | "/exit") {
            break;
        }
        // the reply runs on its own task: when she asks a yes/no question, the loop reads the
        // answer while the reply waits for it
        let (asked_tx, mut asked) = tokio::sync::mpsc::unbounded_channel::<()>();
        let chat = chat.clone();
        let mut reply = tokio::spawn(async move {
            let mut started = false;
            chat.send(&line, |e| match e {
                ChatEvent::Thinking | ChatEvent::Body(_) => {}
                ChatEvent::Delta(d) => {
                    if !started {
                        print!("capri > ");
                        started = true;
                    }
                    print!("{d}");
                    let _ = std::io::stdout().flush();
                }
                ChatEvent::Confirm(q) => {
                    if started {
                        println!();
                        started = false;
                    }
                    println!("capri ? {q}");
                    let _ = asked_tx.send(());
                }
                ChatEvent::Done(full) => {
                    if started {
                        println!();
                    } else {
                        println!("capri > {full}");
                    }
                }
                ChatEvent::Failed(err) => println!("[brain offline] {}", err.detail),
            })
            .await;
        });
        tokio::select! {
            _ = &mut reply => {}
            Some(()) = asked.recv() => {}
        }
    }
    Ok(())
}
