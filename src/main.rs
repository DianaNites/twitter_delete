use std::{
    fs,
    io::{stdout, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::{Parser, ValueHint};
use diesel::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
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

use crate::{
    db::{checked, count_tweets, created_before, deleted, existing},
    models::Tweet as MTweet,
    schema::tweets as tdb,
    twitter::{
        collect_tweets,
        delete_tweets,
        lookup_tweets,
        DeleteResp,
        LookupResp,
        RateLimit,
        TWITTER_DATE,
    },
};

mod config;
mod db;
mod models;
mod schema;
mod twitter;
mod util;

/// Twitter API keys, in a simple JSON format
///
/// See [Access]
static ACCESS: &str = include_str!("../scratch/access.json");

static HUMAN_TIME: &[FormatItem] = format_description!("[hour repr:12]:[minute]:[second] [period]");

/// Twitter API keys.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub struct Access {
    // test_path: PathBuf,
    api_key: String,
    api_secret: String,
    access: String,
    access_secret: String,
}

/// Parse tweets from your twitter archive
#[derive(Parser, Debug)]
enum Args {
    /// Import tweets from the twitter archive for processing
    ///
    /// Tweets are imported into a local database at
    /// `$HOME/.config/twitter_delete/tweets.db`
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

        /// Don't delete tweets if they have at least this many likes.
        ///
        /// WARNING, this is based on likes in your imported twitter archive.
        /// This DOES NOT check for the latest information on twitter
        #[clap(long, short = 'l', value_hint = ValueHint::Other, default_value = "0")]
        unless_likes: u32,

        /// Don't delete tweets if they have at least this many retweets.
        ///
        /// WARNING, this is based on retweets in your imported twitter archive.
        /// This DOES NOT check for the latest information on twitter
        #[clap(long, short = 'r', value_hint = ValueHint::Other, default_value = "0")]
        unless_retweets: u32,
    },

    /// Show information about tweets in the database
    Stats {
        //
    },
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

    let args = Args::parse();

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
                .format(HUMAN_TIME)?
        );

        Ok(())
    };
    let mut stdout = stdout().lock();

    let progress_style = ProgressStyle::with_template(
        //
        "{msg}\n[{elapsed_precise}] {wide_bar} {pos:>7}/{len:7} ({percent}%) \nETA: {eta_precise}\n{prefix}",
    )
    .unwrap();

    match args {
        Args::Import { path } => {
            let added = import_tweets(conn, &path)?;
            writeln!(
                stdout,
                "Imported {added} tweets. Total tweets {}",
                count_tweets(conn)?
            )?;

            // Lookup `tweets` on twitter and mark the ones that are already
            // deleted
            //
            // This skips tweets that have already been checked
            let unchecked_tweets: Vec<String> = tdb::dsl::tweets
                .order(tdb::dsl::id_str.asc())
                .filter(existing())
                .select(tdb::dsl::id_str)
                .load::<String>(conn)?;

            writeln!(
                stdout,
                "Checking whether {} tweets were already deleted, out of {} total tweets",
                unchecked_tweets.len(),
                count_tweets(conn)?
            )?;

            let mut total = 0;

            let pb = ProgressBar::new(unchecked_tweets.len() as u64);
            pb.set_message("Checking whether tweets exist");
            pb.set_style(progress_style);
            pb.tick();

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

                    // Advance progress bar
                    pb.inc(100);
                    pb.set_prefix(format!("Marked {gone} tweets as already deleted"));

                    Ok(())
                },
            )?;
            pb.finish();
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
                .filter(diesel::dsl::not(tdb::dsl::id_str.eq_any(&exclude)))
                .filter(tdb::dsl::likes.lt(unless_likes as i32))
                .filter(tdb::dsl::retweets.lt(unless_retweets as i32))
                .select(tdb::dsl::id_str)
                .load::<String>(conn)?;

            let pb = ProgressBar::new(to_process.len() as u64);
            pb.set_message("Deleting tweets");
            pb.set_style(progress_style);
            pb.tick();

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

                    pb.inc(1);
                    pb.set_prefix(format!("Deleted tweet {id}"));

                    Ok(())
                },
            )?;
            pb.finish();
            writeln!(stdout, "Deleted {total} tweets")?;
        }
        Args::Stats {} => {
            writeln!(
                stdout,
                "\
Total Imported Tweets: {}
Deleted Tweets: {}
Checked* Tweets: {}

*During Twitter Archive importing, tweets are checked for whether they've already
been deleted or not. If this process was not interrupted, this is the same as the total tweets.
",
                count_tweets(conn)?,
                tdb::dsl::tweets
                    .filter(tdb::dsl::deleted.eq(true))
                    .count()
                    .get_result::<i64>(conn)?,
                tdb::dsl::tweets
                    .filter(tdb::dsl::checked.eq(true))
                    .count()
                    .get_result::<i64>(conn)?,
            )?;
        }
    };

    Ok(())
}
