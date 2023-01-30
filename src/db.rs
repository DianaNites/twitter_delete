//! Handles interfacing with the tweets database

use std::path::Path;

use anyhow::{anyhow, Result};
use diesel::{
    dsl::{Asc, Eq, Filter, Lt, Order},
    prelude::*,
    result::Error as DieselError,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::{models::Tweet, schema::tweets as db};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

type ExistingDeleted = Filter<db::dsl::tweets, Eq<db::dsl::deleted, bool>>;
type ExistingFilter = Filter<ExistingDeleted, Eq<db::dsl::checked, bool>>;

pub type Existing = Order<ExistingFilter, Asc<db::dsl::id_str>>;
type CreatedBeforeFilter = Filter<db::dsl::tweets, Lt<db::dsl::created_at, i64>>;
pub type CreatedBefore = Order<CreatedBeforeFilter, Asc<db::dsl::id_str>>;

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

/// Gets all tweets created before `utc`
///
/// Uses UTC unix time.
///
/// In ascending/alphabetical/lexicographical order
pub fn created_before(utc: i64) -> CreatedBefore {
    use db::dsl::*;
    tweets.filter(created_at.lt(utc)).order(id_str.asc())
}

/// Gets all existing, not marked as deleted, tweets, that haven't been checked
/// already
///
/// In ascending/alphabetical/lexicographical order
pub fn existing() -> Existing {
    use db::dsl::*;
    tweets
        .filter(deleted.eq(false))
        .filter(checked.eq(false))
        .order(id_str.asc())
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
        // TODO: use between?
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
        // TODO: use between?
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
