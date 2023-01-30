//! Handles interfacing with the tweets database

use std::path::Path;

use anyhow::{anyhow, Result};
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::{models::Tweet, schema::tweets as db};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

/// Create or open a database at `db_path`
///
/// Runs any pending migrations
pub fn create_db(db_path: &Path) -> Result<SqliteConnection> {
    let db_path = db_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid UTF-8 in database path {}", db_path.display()))?;
    let mut conn = SqliteConnection::establish(db_path)?;

    conn.run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow!(e))?;
    Ok(conn)
}

/// Add tweets to the database, returning how many were added
///
/// Ignores duplicate tweets, as determined by the tweet ID
pub fn add_tweets(conn: &mut SqliteConnection, tweets: &[Tweet]) -> Result<usize> {
    let added = diesel::insert_or_ignore_into(db::table)
        .values(tweets)
        .execute(conn)?;
    Ok(added)
}

/// Return how many tweets there are in the database
pub fn count_tweets(conn: &mut SqliteConnection) -> Result<i64> {
    let c = db::dsl::tweets.count().get_result::<i64>(conn)?;
    Ok(c)
}
