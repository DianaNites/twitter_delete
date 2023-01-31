#![allow(
    unused_imports,
    dead_code,
    unreachable_code,
    unused_mut,
    unused_variables,
    clippy::let_and_return,
    clippy::redundant_clone,
    clippy::never_loop
)]
use std::{
    collections::HashMap,
    fs,
    io::{stdout, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::{Parser, ValueHint};
use diesel::prelude::*;
use reqwest::blocking::{ClientBuilder, Response};
use serde::Deserialize;
use serde_json::from_str;
use time::{
    format_description::FormatItem,
    macros::format_description,
    Duration,
    OffsetDateTime,
    PrimitiveDateTime,
    UtcOffset,
};
use twitter::Tweet;

use crate::{
    db::{checked, count_tweets, created_before, deleted, existing},
    models::Tweet as MTweet,
    schema::tweets as tdb,
    twitter::{collect_tweets, delete_tweets, lookup_tweets, DeleteResp, LookupResp, RateLimit},
};

mod config;
mod db;
mod models;
mod schema;
mod twitter;
mod util;

static ACCESS: &str = include_str!("../scratch/access.json");

static TWITTER_DATE: &[FormatItem] = format_description!(
    "[weekday repr:short case_sensitive:false] [month repr:short] [day] [hour]:[minute]:[second] +0000 [year]"
);

/// Parse tweets from your twitter archive
#[derive(Parser, Debug)]
enum Args {
    /// Import tweets from the twitter archive for processing
    Import {
        /// Path to your twitter archive
        ///
        /// This is the folder with "Your archive.html" in it.
        #[clap(value_hint = ValueHint::DirPath)]
        path: PathBuf,
    },

    /// Delete tweets that have been imported, subject to the provided filters
    ///
    /// Without any filters this will do nothing, as a precaution against
    /// accidental deletions.
    ///
    /// If you really want to delete ***ALL*** tweets, pass in `--older_than 0`
    Delete {
        /// Exclude these tweet IDs
        #[clap(long, short, value_delimiter = ',', value_hint = ValueHint::Other)]
        exclude: Vec<String>,

        /// Delete tweets older than this many days
        #[clap(long, short, value_hint = ValueHint::Other)]
        older_than: u32,

        /// Don't delete tweets if they have at least this many likes
        #[clap(long, short = 'l', value_hint = ValueHint::Other, default_value = "0")]
        unless_likes: u32,

        /// Don't delete tweets if they have at least this many retweets
        #[clap(long, short = 'r', value_hint = ValueHint::Other, default_value = "0")]
        unless_retweets: u32,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE", deny_unknown_fields)]
pub struct Access {
    test_path: PathBuf,
    api_key: String,
    api_secret: String,
    access: String,
    access_secret: String,
}

/// Import tweets from the twitter archive to our database
///
/// Ignores any tweets already in the database
fn import_tweets(conn: &mut SqliteConnection, path: &Path) -> Result<usize> {
    let tweets = collect_tweets(path)?;

    let tweets: Vec<MTweet> = tweets
        .into_iter()
        .map(|tw| {
            // Unwrap should only fail if twitter archive is bad/evil
            // Also `?` cant be used here
            MTweet::new(
                tw.id_str,
                tw.retweets.parse().unwrap(),
                tw.likes.parse().unwrap(),
                PrimitiveDateTime::parse(&tw.created_at, TWITTER_DATE)
                    .unwrap()
                    .assume_utc()
                    .unix_timestamp(),
            )
        })
        .collect();
    let added = db::add_tweets(conn, &tweets)?;

    Ok(added)
}

fn main() -> Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("Missing $HOME"))?;
    let config_path = Path::new(&home).join(".config/twitter_delete");
    let db_path = config_path.join("tweets.db");
    let utc_offset = UtcOffset::current_local_offset()?;

    fs::create_dir_all(config_path)?;
    let keys: Access = from_str(ACCESS)?;

    let mut args = Args::parse();
    dbg!(&args);

    let mut conn = crate::db::create_db(&db_path)?;
    let conn = &mut conn;

    let client = ClientBuilder::new().build()?;

    let mut rate_limited = |limit, _res: &Response| {
        let secs = match limit {
            RateLimit::Until(secs) => secs,
            RateLimit::Unknown => 60 * 15,
        } as i64;
        let secs = secs
            .checked_sub(OffsetDateTime::now_utc().unix_timestamp())
            .unwrap_or(60 * 15);

        eprintln!(
            "Rate limited, waiting until {} ({secs} seconds)",
            (OffsetDateTime::now_utc() + Duration::seconds(secs))
                .to_offset(utc_offset)
                .time()
                .format(format_description!(
                    "[hour repr:12]:[minute]:[second] [period]"
                ))?
        );

        Ok(())
    };
    let mut stdout = stdout().lock();

    match args {
        Args::Import { path } => {
            let added = import_tweets(conn, &path)?;
            println!(
                "Loaded {added} tweets. Total tweets {}",
                count_tweets(conn)?
            );

            // Lookup `tweets` on twitter and mark the ones that are already
            // deleted
            //
            // This skips tweets that have already been checked
            let unchecked_tweets: Vec<String> = tdb::dsl::tweets
                .order(tdb::dsl::id_str.asc())
                .filter(existing())
                .select(tdb::dsl::id_str)
                .load::<String>(conn)?;

            println!(
                "Checking whether {} tweets were already deleted, out of {} total tweets",
                unchecked_tweets.len(),
                count_tweets(conn)?
            );

            // Last `gone` and count of equal values
            let mut last = (0, 0);
            let mut total = 0;

            lookup_tweets(
                &client,
                &keys,
                unchecked_tweets.iter().map(|f| f.as_str()),
                &mut rate_limited,
                |res| {
                    let res: LookupResp = res.json()?;
                    let mut ids: Vec<&str> = res
                        .id
                        .iter()
                        .filter(|(_, v)| v.is_none())
                        .map(|(k, _)| k.as_str())
                        .collect();
                    // Make sure its sorted
                    ids.sort();

                    let gone = conn.transaction::<_, anyhow::Error, _>(|conn| {
                        // Mark all tweets as checked
                        checked(conn, res.id.keys().map(|k| k.as_str()))?;
                        let gone = deleted(conn, ids.iter().copied())?;
                        Ok(gone)
                    })?;
                    total += gone;
                    if gone == last.0 {
                        last.1 += 1;
                    } else {
                        writeln!(stdout)?;
                        last = (gone, 1);
                    }

                    if last.1 > 1 {
                        // TODO: https://github.com/console-rs/indicatif
                        write!(
                            stdout,
                            "Marked {gone} x{} tweets as already deleted from twitter\r",
                            last.1
                        )?;
                    } else {
                        write!(
                            stdout,
                            "Marked {gone} tweets as already deleted from twitter\r"
                        )?;
                    }
                    stdout.flush()?;

                    Ok(())
                },
            )?;
            writeln!(
                stdout,
                "Marked {total} total tweets as already deleted from twitter"
            )?;
        }
        Args::Delete {
            exclude,
            older_than,
            unless_likes,
            unless_retweets,
        } => {
            let off = Duration::days(older_than.into());
            let off = OffsetDateTime::now_utc().checked_sub(off).ok_or_else(|| {
                anyhow!(
                    "Specified offset of {} ({off}) is too far in the past",
                    util::human_dur(off),
                )
            })?;
            let off = off.unix_timestamp();

            let to_process: Vec<String> = tdb::dsl::tweets
                .order(tdb::dsl::id_str.asc())
                .filter(created_before(off))
                .filter(existing())
                .select(tdb::dsl::id_str)
                .load::<String>(conn)?;
            println!(
                "Deleting {} tweets, out of {} total tweets",
                to_process.len(),
                count_tweets(conn)?
            );

            let mut total = 0;

            delete_tweets(
                &client,
                &keys,
                to_process.iter().map(|f| f.as_str()),
                &mut rate_limited,
                |res| {
                    let res: DeleteResp = res.json()?;
                    let id = res.id_str;

                    total += deleted(conn, [id.as_str()].iter().copied())?;

                    // TODO: https://github.com/console-rs/indicatif
                    writeln!(stdout, "Deleted tweet {id}")?;

                    Ok(())
                },
            )?;
            writeln!(stdout, "Deleted {total} tweets")?;
        }
    };

    Ok(())
}
