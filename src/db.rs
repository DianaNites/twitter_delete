//! Handles interfacing with the tweets database

use std::path::Path;

use anyhow::{anyhow, Result};
use diesel::{
    dsl::{sql, And, Eq, Lt},
    prelude::*,
    result::Error as DieselError,
    sql_types::Untyped,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::{
    models::Tweet,
    schema::{accounts as adb, tweets as db},
};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

type ExistingDeleted = Eq<db::dsl::deleted, bool>;
type ExistingChecked = Eq<db::dsl::checked, bool>;
type CreatedBeforeAt = Lt<db::dsl::created_at, i64>;

pub type Existing = And<ExistingDeleted, ExistingChecked>;
pub type CreatedBefore = CreatedBeforeAt;

/// Create or open a database at `db_path`
///
/// Runs any pending migrations
pub fn create_db(db_path: &Path) -> Result<SqliteConnection> {
    let db_path = db_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid UTF-8 in database path {}", db_path.display()))?;
    let mut conn = SqliteConnection::establish(db_path)?;
    sql::<Untyped>("PRAGMA foreign_keys = ON;").execute(&mut conn)?;

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

/// Return how many accounts there are in the database
///
/// Does not include the "default" unknown account
pub fn _count_accounts(conn: &mut SqliteConnection) -> Result<i64> {
    let c = adb::dsl::accounts.count().get_result::<i64>(conn)?;
    Ok(c)
}

/// Gets all tweets created before `utc`
///
/// Uses UTC unix time.
pub fn created_before(utc: i64) -> CreatedBefore {
    use db::dsl::*;
    created_at.lt(utc)
}

/// Gets all existing tweets, meaning not marked as deleted and not already
/// checked?
pub fn existing() -> Existing {
    use db::dsl::*;
    deleted.eq(false).and(checked.eq(false))
}

/// Mark `tweets` as checked, returning how many were marked
///
/// This all occurs in a single transaction.
///
/// It is a logic error for `tweets` not to be in sorted order
pub fn checked<'a>(
    conn: &mut SqliteConnection,
    tweets: impl Iterator<Item = &'a str>,
) -> Result<usize> {
    let gone = conn.transaction::<_, DieselError, _>(|conn| {
        let mut gone = 0;
        // TODO: use range of some sort?
        for tweet in tweets {
            use db::dsl::*;
            gone += diesel::update(tweets.find(tweet))
                .set(checked.eq(true))
                .execute(conn)?;
        }
        Ok(gone)
    })?;
    Ok(gone)
}

/// Mark `tweets` as deleted, returning how many were marked
///
/// This all occurs in a single transaction.
///
/// It is a logic error for `tweets` not to be in sorted order
pub fn deleted<'a>(
    conn: &mut SqliteConnection,
    tweets: impl Iterator<Item = &'a str>,
) -> Result<usize> {
    let gone = conn.transaction::<_, DieselError, _>(|conn| {
        let mut gone = 0;
        // TODO: use range of some sort?
        for tweet in tweets {
            use db::dsl::*;
            gone += diesel::update(tweets.find(tweet))
                .set(deleted.eq(true))
                .execute(conn)?;
        }
        Ok(gone)
    })?;
    Ok(gone)
}
