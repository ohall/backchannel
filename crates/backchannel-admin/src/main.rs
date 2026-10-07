use anyhow::Context;
use backchannel_core::{db, token};
use clap::{Parser, Subcommand};
use std::env;

#[derive(Parser)]
#[command(name = "backchannel-admin")]
#[command(about = "Backchannel administrative CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new admin token
    GenerateAdminToken,

    /// Clean old rate limit buckets
    CleanRateLimitBuckets {
        /// Age threshold in minutes (default: 120)
        #[arg(short, long, default_value = "120")]
        minutes: i32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::GenerateAdminToken => {
            let token = token::generate_token()
                .map_err(|e| anyhow::anyhow!("Failed to generate token: {}", e))?;
            let hash = token::hash_token(&token);

            println!("Generated admin token:");
            println!("Token: {}", token);
            println!("SHA-256 Hash: {}", hash);
            println!();
            println!("Set ADMIN_TOKEN_SHA256={}", hash);
            println!("WARNING: The token is shown only once. Store it securely.");
        }
        Commands::CleanRateLimitBuckets { minutes } => {
            let database_url =
                env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;

            let pool = db::create_pool(&database_url)
                .await
                .context("Failed to connect to database")?;

            let threshold = chrono::Utc::now() - chrono::Duration::minutes(minutes as i64);
            let threshold_str = threshold.format("%Y-%m-%d %H:%M").to_string();

            let result = sqlx::query("DELETE FROM rate_limit_buckets WHERE minute_bucket < $1")
                .bind(&threshold_str)
                .execute(&pool)
                .await
                .context("Failed to clean rate limit buckets")?;

            tracing::info!(
                "Cleaned {} rate limit bucket(s) older than {} minutes",
                result.rows_affected(),
                minutes
            );
        }
    }

    Ok(())
}
