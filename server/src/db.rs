use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use diesel::PgConnection;
use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::{Build, Rocket};

use crate::config::Config;

pub type DbPool = Pool<ConnectionManager<PgConnection>>;
pub type DbConn = PooledConnection<ConnectionManager<PgConnection>>;

pub struct Database(pub DbPool);

impl Database {
    pub fn new(database_url: &str) -> Result<Self, diesel::r2d2::PoolError> {
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = Pool::builder().max_size(32).build(manager)?;
        Ok(Database(pool))
    }

    pub fn get(&self) -> Result<DbConn, diesel::r2d2::PoolError> {
        self.0.get()
    }
}

pub struct DbFairing {
    database_url: String,
}

impl DbFairing {
    pub fn new(config: &Config) -> Self {
        Self {
            database_url: config.db.connection.clone(),
        }
    }
}

#[rocket::async_trait]
impl Fairing for DbFairing {
    fn info(&self) -> Info {
        Info {
            name: "Database Pool",
            kind: Kind::Ignite,
        }
    }

    async fn on_ignite(&self, rocket: Rocket<Build>) -> rocket::fairing::Result {
        match Database::new(&self.database_url) {
            Ok(db) => Ok(rocket.manage(db)),
            Err(e) => {
                tracing::error!("Failed to initialize database pool: {}", e);
                Err(rocket)
            }
        }
    }
}

// Request guard for database connections
pub struct Db(pub DbConn);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Db {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> Outcome<Self, Self::Error> {
        let database = request.rocket().state::<Database>();
        match database {
            Some(db) => match db.get() {
                Ok(conn) => Outcome::Success(Db(conn)),
                Err(_) => Outcome::Error((Status::ServiceUnavailable, ())),
            },
            None => Outcome::Error((Status::InternalServerError, ())),
        }
    }
}

// Run migrations
pub fn run_migrations(conn: &mut PgConnection) {
    use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

    const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

    conn.run_pending_migrations(MIGRATIONS)
        .expect("Failed to run migrations");
}
