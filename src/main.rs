use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "ngasak")]
#[command(author = "thestepashich")]
#[command(version = "0.1.0")]
#[command(about = "NgaSak - Ngate + Sakura automation CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: ngasak::cli::commands::Commands,

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

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    match cli.command {
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
    }
}
