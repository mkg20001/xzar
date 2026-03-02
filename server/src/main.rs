#[macro_use]
extern crate rocket;

use std::net::IpAddr;

use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use rocket::fairing::AdHoc;
use rocket::http::Method;
use rocket::tokio;
use rocket_cors::{AllowedOrigins, CorsOptions};
use tracing_subscriber::EnvFilter;

use xzar_server::auth::TokenStore;
use xzar_server::config::Config;
use xzar_server::db::{self, Database};
use xzar_server::gc;
use xzar_server::routes;
use xzar_server::storage::Storage;

#[launch]
async fn rocket() -> _ {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load configuration
    let config_path = std::env::var("XZAR_CONFIG").unwrap_or_else(|_| "config.yaml".to_string());
    let config = Config::load(&config_path).expect("Failed to load configuration");

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

    // Initialize database connection pool
    let database_url = &config.db.connection;
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = Pool::builder()
        .max_size(10)
        .build(manager)
        .expect("Failed to create database pool");

    // Run migrations
    {
        let mut conn = pool.get().expect("Failed to get connection for migrations");
        db::run_migrations(&mut conn);
        tracing::info!("Database migrations completed");
    }

    // Initialize storage
    let storage = Storage::new(&config.storage)
        .await
        .expect("Failed to initialize storage");

    // Initialize token store
    let token_hashes = config.get_token_hashes();
    let token_store = TokenStore::new(token_hashes);

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

    rocket
        .manage(Database(pool))
        .manage(storage)
        .manage(token_store)
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
            ],
        )
}
