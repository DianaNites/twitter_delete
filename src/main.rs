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
use clap::Parser;
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
struct Args {
    /// Path to your twitter archive
    ///
    /// This is the folder with "Your archive.html" in it.
    path: PathBuf,
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

fn main() -> Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("Missing $HOME"))?;
    let config_path = Path::new(&home).join(".config/twitter_delete");
    let db_path = config_path.join("tweets.db");
    let utc_offset = UtcOffset::current_local_offset()?;

    fs::create_dir_all(config_path)?;
    let keys: Access = from_str(ACCESS)?;

    let tweets = collect_tweets(&keys.test_path)?;

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

    let mut conn = crate::db::create_db(&db_path)?;

    // Add tweets to db, ignoring ones already there
    let added = db::add_tweets(&mut conn, &tweets)?;

    println!(
        "Loaded {added} tweets. Total tweets {}",
        count_tweets(&mut conn)?
    );

    // NOTE: Test select tweets older than 360 days
    // My test archive is already older than 30 days lol
    let off = Duration::days(360);
    let off = OffsetDateTime::now_utc().checked_sub(off).ok_or_else(|| {
        anyhow!(
            "Specified offset of {} ({off}) is too far in the past",
            util::human_dur(off),
        )
    })?;
    let off = off.unix_timestamp();

    // Find all tweets older than the provided offset, delete them,
    // and mark as deleted

    let conn = &mut conn;

    let client = ClientBuilder::new().build()?;

    // Lookup tweets in the DB and mark them as deleted if they don't exist
    // Skips tweets we have already checked
    let unchecked_tweets: Vec<MTweet> = tdb::dsl::tweets
        .order(tdb::dsl::id_str.asc())
        .filter(created_before(off))
        .filter(existing())
        .load::<MTweet>(conn)?;

    println!(
        "Checking whether {} tweets were already deleted, out of {} total tweets",
        unchecked_tweets.len(),
        count_tweets(conn)?
    );

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
    // Last `gone` and count of equal values
    let mut last = (0, 0);
    let mut total = 0;

    lookup_tweets(
        &client,
        &keys,
        unchecked_tweets.iter().map(|f| f.id_str.as_str()),
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

    let to_process: Vec<MTweet> = tdb::dsl::tweets
        .order(tdb::dsl::id_str.asc())
        .filter(created_before(off))
        .filter(existing())
        .load::<MTweet>(conn)?;
    println!(
        "Deleting {} tweets, out of {} total tweets",
        to_process.len(),
        count_tweets(conn)?
    );

    let mut total = 0;

    delete_tweets(
        &client,
        &keys,
        to_process.iter().map(|f| f.id_str.as_str()),
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

    // let mut args = Args::parse();
    Ok(())
}
