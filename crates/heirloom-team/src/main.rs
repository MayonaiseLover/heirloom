//! `heirloom-team-server` — runnable HTTP server for shared team memory.

use anyhow::Result;
use clap::{Parser, Subcommand};
use heirloom_team::db::{Db, Role};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(version, about = "Self-hostable team-memory server for Heirloom.", long_about = None)]
struct Cli {
    /// Bind address (default 127.0.0.1:7900).
    #[arg(long, default_value = "127.0.0.1:7900")]
    addr: String,

    /// Directory for the SQLite database and audit logs.
    #[arg(long, default_value = "./heirloom-team-data")]
    data_dir: PathBuf,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run admin operations against the local database without starting the server.
    Admin {
        #[command(subcommand)]
        action: Admin,
    },
}

#[derive(Subcommand, Debug)]
enum Admin {
    /// Initialize the database, create the first team, and mint the first admin token.
    Init {
        #[arg(long)]
        team_name: String,
        #[arg(long, default_value = "admin")]
        admin_name: String,
    },
    /// Create a member; prints their bearer token (shown once).
    AddMember {
        #[arg(long)]
        name: String,
        /// admin | member | readonly (default: member)
        #[arg(long, default_value = "member")]
        role: String,
    },
    /// List members.
    ListMembers,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing();

    std::fs::create_dir_all(&cli.data_dir)?;
    let db_path = cli.data_dir.join("team.db");
    let db = Arc::new(Db::open(&db_path)?);

    match cli.cmd {
        Some(Cmd::Admin { action }) => run_admin(&db, action),
        None => {
            let addr: std::net::SocketAddr = cli.addr.parse()?;
            heirloom_team::serve(db, addr).await
        }
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("HEIRLOOM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn run_admin(db: &Arc<Db>, action: Admin) -> Result<()> {
    match action {
        Admin::Init {
            team_name,
            admin_name,
        } => {
            if let Some(team) = db.first_team()? {
                anyhow::bail!(
                    "team already exists: {} ({}). Use add-member instead.",
                    team.name,
                    team.id
                );
            }
            let team = db.create_team(&team_name)?;
            let (member, token) = db.create_member(&team.id, &admin_name, Role::Admin)?;
            println!("✓ initialized team");
            println!("  team:    {} ({})", team.name, team.id);
            println!("  admin:   {} ({})", member.name, member.id);
            println!();
            println!("Bootstrap token (copy now, will not be shown again):");
            println!();
            println!("    {}", token);
            println!();
            println!("Give this to the admin so they can run:");
            println!(
                "    heirloom team join http://your-server-host:7900 --token {}",
                token
            );
            Ok(())
        }
        Admin::AddMember { name, role } => {
            let team = db
                .first_team()?
                .ok_or_else(|| anyhow::anyhow!("no team exists yet — run `admin init` first"))?;
            let role = match role.as_str() {
                "admin" => Role::Admin,
                "readonly" => Role::ReadOnly,
                _ => Role::Member,
            };
            let (member, token) = db.create_member(&team.id, &name, role)?;
            println!("✓ added member");
            println!(
                "  member:  {} ({}, {})",
                member.name,
                member.id,
                role.to_db()
            );
            println!();
            println!("Token (copy now, will not be shown again):");
            println!();
            println!("    {}", token);
            println!();
            println!(
                "They join with: heirloom team join http://your-server-host:7900 --token <token>"
            );
            Ok(())
        }
        Admin::ListMembers => {
            let team = match db.first_team()? {
                Some(t) => t,
                None => {
                    println!("(no team yet — run `admin init`)");
                    return Ok(());
                }
            };
            let members = db.list_members(&team.id)?;
            println!("Team: {} ({})", team.name, team.id);
            for m in members {
                println!(
                    "  • {:<24} {:<10} {} {}",
                    m.name,
                    m.role.to_db(),
                    m.id,
                    m.created_at.format("%Y-%m-%d")
                );
            }
            Ok(())
        }
    }
}
