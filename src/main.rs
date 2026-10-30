use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "ngasak")]
#[command(author = "thestepashich")]
#[command(version = "0.1.0")]
#[command(about = "NgaSak - Ngate + Sakura automation CLI", long_about = None)]
struct Cli {
    // Optional so the arg-less launch (double-click on Windows) defaults to
    // the full `run` flow instead of printing help and evaporating.
    #[command(subcommand)]
    command: Option<ngasak::cli::commands::Commands>,

    #[arg(short, long, global = true)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        "ngasak=debug,zbus=warn,reqwest=warn"
    } else {
        "ngasak=info"
    };

    // ANSI escape codes render as raw garbage (`←[2m`) on legacy Windows
    // consoles; keep colors where the terminal actually understands them.
    #[allow(unused_mut)]
    let mut fmt = tracing_subscriber::fmt::layer();
    #[cfg(target_os = "windows")]
    let fmt = fmt.with_ansi(false);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with(fmt)
        .init();

    // No subcommand = the double-click path: run the whole flow and hold the
    // console window open afterwards so the outcome stays readable.
    let bare = cli.command.is_none();
    let result = match cli.command {
        None => {
            println!("No subcommand given - running the full flow (ngasak run)");
            ngasak::cli::commands::cmd_run(
                ngasak::cli::commands::RunArgs::default(),
            )
            .await
        }
        Some(command) => match command {
            ngasak::cli::commands::Commands::Run(args) => {
                ngasak::cli::commands::cmd_run(args).await
            }
            ngasak::cli::commands::Commands::Ngate(args) => {
                ngasak::cli::commands::cmd_ngate(args).await
            }
            ngasak::cli::commands::Commands::Sakura(args) => {
                ngasak::cli::commands::cmd_sakura(args).await
            }
            ngasak::cli::commands::Commands::LoginEr(args) => {
                ngasak::cli::commands::cmd_login_er(args).await
            }
            ngasak::cli::commands::Commands::Config(args) => {
                ngasak::cli::commands::cmd_config(args).await
            }
        },
    };

    if bare {
        match &result {
            Ok(()) => println!("NgaSak flow finished."),
            Err(e) => println!("NgaSak flow failed: {e:#}"),
        }
        println!("Press Enter to close...");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        result
    } else {
        result
    }
}
