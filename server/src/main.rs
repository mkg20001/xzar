#[macro_use]
extern crate rocket;

use std::net::IpAddr;

use clap::{Parser, Subcommand};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use rocket::fairing::AdHoc;
use rocket::http::Method;
use rocket::tokio;
use rocket_cors::{AllowedOrigins, CorsOptions};
use tracing_subscriber::EnvFilter;

use xzar_server::auth::hash_token;
use xzar_server::config::Config;
use xzar_server::db::{self, Database};
use xzar_server::gc;
use xzar_server::models::{NewToken, NewUser, Token, User};
use xzar_server::routes;
use xzar_server::schema::{tokens, users};
use xzar_server::storage::Storage;

/// xzar-server - Nix binary cache server
#[derive(Parser, Debug)]
#[command(name = "xzar-server", version, about)]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "config.yaml")]
    config: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the server (default)
    Serve,

    /// Run garbage collection once and exit
    Gc,

    /// User management commands
    User {
        #[command(subcommand)]
        action: UserAction,
    },

    /// Token management commands
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
}

#[derive(Subcommand, Debug)]
enum UserAction {
    /// Create a new user
    Create {
        /// Username
        name: String,
        /// Make user an admin
        #[arg(long)]
        admin: bool,
    },
    /// List all users
    List,
    /// Delete a user
    Delete {
        /// Username
        name: String,
    },
    /// Promote user to admin
    Promote {
        /// Username
        name: String,
    },
    /// Demote admin to regular user
    Demote {
        /// Username
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum TokenAction {
    /// Create a new token
    Create {
        /// User to create token for (omit for system token)
        #[arg(long)]
        user: Option<String>,
        /// Create a system token (requires --user to be omitted)
        #[arg(long)]
        system: bool,
        /// Description for the token
        #[arg(long)]
        description: Option<String>,
    },
    /// List all tokens
    List,
    /// Revoke a token
    Revoke {
        /// Token ID
        id: i32,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    let args = Args::parse();

    // Load configuration
    let config_path =
        std::env::var("XZAR_CONFIG").unwrap_or_else(|_| args.config.clone());
    let config = Config::load(&config_path)?;

    // Initialize database pool
    let manager = ConnectionManager::<PgConnection>::new(&config.db.connection);
    let pool = Pool::builder().max_size(10).build(manager)?;

    // Run migrations
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn);
    }

    match args.command.unwrap_or(Command::Serve) {
        Command::Serve => run_server(config, pool).await,
        Command::Gc => run_gc_once(config, pool).await,
        Command::User { action } => {
            handle_user_action(pool, action)?;
            Ok(())
        }
        Command::Token { action } => {
            handle_token_action(pool, action)?;
            Ok(())
        }
    }
}

fn handle_user_action(
    pool: Pool<ConnectionManager<PgConnection>>,
    action: UserAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = pool.get()?;

    match action {
        UserAction::Create { name, admin } => {
            let new_user = NewUser {
                name: name.clone(),
                is_admin: admin,
            };

            let user: User = diesel::insert_into(users::table)
                .values(&new_user)
                .get_result(&mut conn)?;

            println!(
                "Created user '{}' (id: {}, admin: {})",
                user.name, user.id, user.is_admin
            );
        }

        UserAction::List => {
            let all_users: Vec<User> = users::table.order(users::id.asc()).load(&mut conn)?;

            println!("{:<6} {:<30} {:<8} {}", "ID", "Name", "Admin", "Created");
            println!("{}", "-".repeat(60));
            for user in all_users {
                println!(
                    "{:<6} {:<30} {:<8} {}",
                    user.id, user.name, user.is_admin, user.created
                );
            }
        }

        UserAction::Delete { name } => {
            let deleted =
                diesel::delete(users::table.filter(users::name.eq(&name))).execute(&mut conn)?;

            if deleted > 0 {
                println!("Deleted user '{}'", name);
            } else {
                eprintln!("User '{}' not found", name);
                std::process::exit(1);
            }
        }

        UserAction::Promote { name } => {
            let updated = diesel::update(users::table.filter(users::name.eq(&name)))
                .set(users::is_admin.eq(true))
                .execute(&mut conn)?;

            if updated > 0 {
                println!("Promoted '{}' to admin", name);
            } else {
                eprintln!("User '{}' not found", name);
                std::process::exit(1);
            }
        }

        UserAction::Demote { name } => {
            let updated = diesel::update(users::table.filter(users::name.eq(&name)))
                .set(users::is_admin.eq(false))
                .execute(&mut conn)?;

            if updated > 0 {
                println!("Demoted '{}' from admin", name);
            } else {
                eprintln!("User '{}' not found", name);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn handle_token_action(
    pool: Pool<ConnectionManager<PgConnection>>,
    action: TokenAction,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = pool.get()?;

    match action {
        TokenAction::Create {
            user,
            system,
            description,
        } => {
            // Validate: either user token or system token, not both
            if user.is_some() && system {
                eprintln!("Cannot specify both --user and --system");
                std::process::exit(1);
            }

            if user.is_none() && !system {
                eprintln!("Must specify either --user or --system");
                std::process::exit(1);
            }

            // Look up user if specified
            let user_id = if let Some(ref username) = user {
                let u: User = users::table
                    .filter(users::name.eq(username))
                    .first(&mut conn)
                    .map_err(|_| format!("User '{}' not found", username))?;
                Some(u.id)
            } else {
                None
            };

            // Generate random token (32 bytes = 64 hex chars)
            let raw_token: String = {
                use rand::Rng;
                let bytes: [u8; 32] = rand::thread_rng().gen();
                bytes.iter().map(|b| format!("{:02x}", b)).collect()
            };

            let token_hash = hash_token(&raw_token);

            let new_token = NewToken {
                user_id,
                token_hash,
                is_system: system,
                description,
            };

            let token: Token = diesel::insert_into(tokens::table)
                .values(&new_token)
                .get_result(&mut conn)?;

            println!("Created token (id: {})", token.id);
            println!("Token: {}", raw_token);
            println!("\nSave this token - it cannot be recovered!");
        }

        TokenAction::List => {
            let all_tokens: Vec<(Token, Option<User>)> = tokens::table
                .left_join(users::table)
                .order(tokens::id.asc())
                .select((Token::as_select(), Option::<User>::as_select()))
                .load(&mut conn)?;

            println!(
                "{:<6} {:<20} {:<8} {:<20} {}",
                "ID", "User", "System", "Created", "Description"
            );
            println!("{}", "-".repeat(80));

            for (token, user) in all_tokens {
                let user_name = user.map(|u| u.name).unwrap_or_else(|| "-".to_string());
                let desc = token.description.unwrap_or_default();
                println!(
                    "{:<6} {:<20} {:<8} {:<20} {}",
                    token.id,
                    user_name,
                    token.is_system,
                    token.created.format("%Y-%m-%d %H:%M"),
                    desc
                );
            }
        }

        TokenAction::Revoke { id } => {
            let deleted = diesel::delete(tokens::table.find(id)).execute(&mut conn)?;

            if deleted > 0 {
                println!("Revoked token {}", id);
            } else {
                eprintln!("Token {} not found", id);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

async fn run_gc_once(
    config: Config,
    pool: Pool<ConnectionManager<PgConnection>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize storage
    let storage = Storage::new(&config.storage)
        .await
        .expect("Failed to initialize storage");

    println!("Running garbage collection...");
    gc::run_gc(&pool, &storage)
        .await
        .map_err(|e| e as Box<dyn std::error::Error>)?;
    println!("Garbage collection completed.");

    Ok(())
}

async fn run_server(
    config: Config,
    pool: Pool<ConnectionManager<PgConnection>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting xzar-server...");
    tracing::info!("Storage path: {}", config.storage);

    // Initialize Sentry if DSN is provided
    let _sentry_guard = config.sentry_dsn.as_ref().map(|dsn| {
        sentry::init((
            dsn.as_str(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        ))
    });

    // Initialize storage
    let storage = Storage::new(&config.storage)
        .await
        .expect("Failed to initialize storage");

    // Clone for GC task
    let gc_pool = pool.clone();
    let gc_storage = storage.clone();

    // Configure Rocket with host/port from config
    let address: IpAddr = config
        .rocket
        .host
        .parse()
        .expect("Invalid rocket.host address");

    let figment = rocket::Config::figment()
        .merge(("address", address))
        .merge(("port", config.rocket.port));

    // Build Rocket instance
    let mut rocket = rocket::custom(figment);

    // Configure CORS if enabled
    if config.cors.enabled {
        let allowed_origins = if config.cors.origins.is_empty() {
            AllowedOrigins::all()
        } else {
            let origins: Vec<&str> = config.cors.origins.iter().map(|s| s.as_str()).collect();
            AllowedOrigins::some_exact(&origins)
        };

        let cors = CorsOptions::default()
            .allowed_origins(allowed_origins)
            .allowed_methods(
                vec![Method::Get, Method::Post, Method::Delete, Method::Options]
                    .into_iter()
                    .map(From::from)
                    .collect(),
            )
            .allow_credentials(true)
            .to_cors()
            .expect("Failed to create CORS configuration");

        rocket = rocket.attach(cors);
        tracing::info!("CORS enabled");
    }

    let rocket = rocket
        .manage(Database(pool))
        .manage(storage)
        .manage(config.clone())
        .attach(AdHoc::on_liftoff("GC Task", |_| {
            Box::pin(async move {
                // Spawn GC background task
                tokio::spawn(async move {
                    gc::run_gc_loop(gc_pool, gc_storage).await;
                });
            })
        }))
        .mount(
            "/",
            routes![
                routes::nix_cache_info,
                routes::get_narinfo,
                routes::get_nar,
                routes::check_paths,
                routes::lock_request,
                routes::lock_extend,
                routes::lock_clear,
                routes::upload_nar,
                routes::finalize_pin,
                routes::list_pins,
                routes::abandon_pin,
                // Auth info
                routes::get_self,
                // Admin routes
                routes::list_users,
                routes::create_user,
                routes::update_user,
                routes::delete_user,
                routes::list_tokens,
                routes::create_token,
                routes::update_token,
                routes::delete_token,
                // UI routes
                routes::ui_index,
                routes::ui_assets,
            ],
        );

    rocket.launch().await?;

    Ok(())
}
