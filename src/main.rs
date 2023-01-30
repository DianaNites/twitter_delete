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
    fmt::Display,
    fs,
    io::stdout,
    iter::once,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::Parser;
use diesel::{prelude::*, SqliteConnection};
use rand::{
    distributions::{Alphanumeric, DistString},
    prelude::*,
    thread_rng,
};
use reqwest::{blocking::ClientBuilder, header::AUTHORIZATION, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use time::{
    format_description::{
        well_known::{iso8601::Config, Iso8601},
        FormatItem,
    },
    macros::format_description,
    Duration,
    OffsetDateTime,
    PrimitiveDateTime,
    UtcOffset,
};

use crate::{
    db::{count_tweets, created_before, existing},
    models::Tweet as MTweet,
    twitter::{collect_tweets, create_auth},
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

/// Lookup 100 tweet IDs at a time
///
/// https://developer.twitter.com/en/docs/twitter-api/v1/tweets/post-and-engage/api-reference/get-statuses-lookup
const TWEET_LOOKUP_URL: &str = "https://api.twitter.com/1.1/statuses/lookup.json";

/// Delete a tweet
///
/// Ends in `{id}.json`
///
/// https://developer.twitter.com/en/docs/twitter-api/v1/tweets/post-and-engage/api-reference/post-statuses-destroy-id
const TWEET_DESTROY_URL: &str = "https://api.twitter.com/1.1/statuses/destroy/";

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

    {
        use crate::schema::tweets::dsl::*;
        let conn = &mut conn;

        let to_process: Vec<MTweet> = created_before(off).load::<MTweet>(conn)?;

        {
            let mut client = ClientBuilder::new().build()?;

            // Lookup tweets in the DB and mark them as deleted if they don't exist
            let existing_tweets: Vec<MTweet> = existing().load::<MTweet>(conn)?;

            let t = tweets.filter(deleted.eq(false)).load::<MTweet>(conn)?;
            dbg!(t.len());

            // Size of all tweet IDs and commas
            // Tweet IDs are assumed to be 19 characters
            // 100 chunks, 19 ID + 1 comma
            let mut ids = String::with_capacity(100 * 20);
            for tweet in t.chunks(100) {
                ids.clear();
                for t in tweet {
                    ids.push_str(&t.id_str);
                    ids.push(',');
                }
                if tweet.len() > 1 {
                    // Pop last comma
                    ids.pop();
                }

                let params = &[
                    //
                    ("id", ids.as_str()),
                    ("map", "true"),
                ];

                let res = loop {
                    let r = client
                        .post(TWEET_LOOKUP_URL)
                        .header(
                            AUTHORIZATION,
                            create_auth(
                                &keys,
                                TWEET_LOOKUP_URL,
                                Method::POST,
                                &params.map(|f| (f.0.to_owned(), f.1.to_owned())),
                            ),
                        )
                        .form(params)
                        .send()?;
                    if r.status().is_success() {
                        break r;
                    } else if r.status() == StatusCode::TOO_MANY_REQUESTS {
                        if let Some(r) = r
                            .headers()
                            .get("x-rate-limit-reset")
                            .map(|f| f.to_str())
                            .transpose()?
                        {
                            let secs: u64 = r.parse()?;
                            let dur = std::time::Duration::from_secs(secs);
                            // Default to 15 minutes
                            let secs = (secs as i64)
                                .checked_sub(OffsetDateTime::now_utc().unix_timestamp())
                                .unwrap_or(60 * 15);
                            let dur = std::time::Duration::from_secs(secs as u64);
                            eprintln!(
                                "Rate limited, waiting until UTC {} ({secs} seconds)",
                                (OffsetDateTime::now_utc() + Duration::seconds(secs))
                                    // .to_offset(UtcOffset::current_local_offset()?)
                                    .time()
                                    .format(format_description!(
                                        "[hour repr:12]:[minute]:[second] [period]"
                                    ))?
                            );
                            std::thread::sleep(dur);
                        } else {
                            // Try waiting 15 minutes if there was no reset
                            // header
                            eprintln!("Rate limited, waiting 15 minutes");
                            let dur = std::time::Duration::from_secs(60 * 15);
                            std::thread::sleep(dur);
                        }
                    } else if r.status().is_client_error() || r.status().is_server_error() {
                        return Err(anyhow!("Encountered HTTP error {}", r.status()));
                    }
                };
                let res: LookupResp = res.json()?;

                let gone = conn.transaction::<_, diesel::result::Error, _>(|conn| {
                    let mut gone = 0;
                    for t in tweet {
                        if let Some(v) = res.id.get(&t.id_str) {
                            if v.is_none() {
                                gone += diesel::update(tweets.find(&t.id_str))
                                    .set(deleted.eq(true))
                                    .execute(conn)?;
                            }
                        }
                    }
                    Ok(gone)
                })?;
                // writeln!(&mut stdout, "Marked {gone} tweets as already deleted")?;
                println!("Marked {gone} tweets as already deleted");
            }
            // For some reason when I leave this running it keeps ending, but running it
            // again finds more??
            // Is there a limit to how much can be returned by filter at once?
            // FUCK ohhh is it because, duh, not all tweets in my test archive actually
            // *are* deleted So of course rerunning it returns the same ones
            // But wait, then why does rerunning it sometimes still mark tweets as deleted..
            // twitter api limitation where it doesn't distinguish between deleted and
            // unavailable properly?
            // TODO: Maybe don't even bother with this and just try deleting them
            // in the first place, ignoring errors if they dont exist?
            // Plus delete has no rate limit
            dbg!("Deleted all tweets??");
            dbg!(t.len());
            dbg!(t.first());
        }

        // let delete = diesel::update(t).set(deleted.eq(true)).execute(conn)?;
        // dbg!(delete);
    };

    // let mut args = Args::parse();
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct LookupResp {
    id: HashMap<String, Option<LookupTweet>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct LookupTweet {
    /// Tweet ID
    id_str: String,

    /// Number of retweets
    retweet_count: u64,

    /// Number of likes
    #[serde(rename = "favorite_count")]
    #[serde(default)]
    like_count: u64,

    /// Time of tweet
    ///
    /// See [`TWITTER_DATE`]
    created_at: String,
}
