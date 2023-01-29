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
    fmt::Display,
    fs,
    iter::once,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::Parser;
use diesel::{prelude::*, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use rand::{
    distributions::{Alphanumeric, DistString},
    prelude::*,
    thread_rng,
};
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
};

use crate::models::Tweet as MTweet;

mod models;
mod schema;

static ACCESS: &str = include_str!("../scratch/access.json");

static TWITTER_DATE: &[FormatItem] = format_description!(
    "[weekday repr:short case_sensitive:false] [month repr:short] [day] [hour]:[minute]:[second] +0000 [year]"
);

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

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
struct Access {
    test_path: PathBuf,
    api_key: String,
    api_secret: String,
    access: String,
    access_secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Tweet {
    /// Tweet ID
    id_str: String,

    /// Number of retweets
    retweet_count: String,

    /// Number of likes
    #[serde(rename = "favorite_count")]
    like_count: String,

    /// Time of tweet
    ///
    /// See [`TWITTER_DATE`]
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct TweetObj {
    tweet: Tweet,
}

fn collect_tweets(path: &Path) -> Result<Vec<Tweet>> {
    let mut files = Vec::with_capacity(10);
    let path = path.join("data");
    for file in path.read_dir()? {
        let file = file?;
        let ty = file.file_type()?;
        if !ty.is_file() {
            continue;
        }
        let name = file.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| anyhow!("Invalid UTF-8 in filename {:?}", file.file_name()))?;
        if !name.starts_with("tweets") {
            continue;
        }
        files.push(file.path());
    }
    if files.len() > 99 {
        return Err(anyhow!("Too many tweet files, can not handle more than 99"));
    }
    files.sort();

    let mut out = Vec::new();
    for path in files {
        // Twitter puts this nonsense in front of the tweet files
        // Assume there are less than 99 parts.
        // This will work for both single and double digits
        // The full line is  `window.YTD.tweets.part4 = [`
        const PREFIX: &str = "window.YTD.tweets.part99 ";
        let data = fs::read_to_string(path)?;

        let data: Vec<TweetObj> = from_str(&data[PREFIX.len()..])?;
        out.extend(data.into_iter().map(|t| t.tweet));
        // break;
    }

    Ok(out)
}

fn main() -> Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("Missing $HOME"))?;
    let config_path = Path::new(&home).join(".config/twitter_delete");
    let db_path = config_path.join("tweets.db");

    let db_path = db_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid UTF-8 in PATH"))?;
    fs::create_dir_all(config_path)?;
    let keys: Access = from_str(ACCESS)?;

    let tweets = collect_tweets(&keys.test_path)?;

    let tweets: Vec<_> = tweets
        .into_iter()
        .map(|tw| {
            // Unwrap should only fail if twitter archive is bad/evil
            // Also `?` cant be used here
            MTweet::new(
                tw.id_str.parse().unwrap(),
                tw.retweet_count.parse().unwrap(),
                tw.like_count.parse().unwrap(),
                PrimitiveDateTime::parse(&tw.created_at, TWITTER_DATE)
                    .unwrap()
                    .assume_utc()
                    .unix_timestamp(),
            )
        })
        .collect();

    let mut conn = SqliteConnection::establish(db_path)?;

    conn.run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow!(e))?;

    // Add tweets to db, ignoring ones already there
    let added = diesel::insert_or_ignore_into(crate::schema::tweets::table)
        .values(&tweets)
        .execute(&mut conn)?;

    println!("Loaded {added} tweets. Total tweets {}", {
        use crate::schema::tweets::dsl::*;
        tweets.count().get_result::<i64>(&mut conn)?
    });

    // NOTE: Test select tweets older than 30 days
    let off = Duration::days(120);
    let off = OffsetDateTime::now_utc().checked_sub(off).ok_or_else(|| {
        anyhow!("Specified offset of {} is too far in the past", {
            if off.whole_weeks() > 52 {
                format!("{} years", off.whole_weeks() / 52)
            } else if off.whole_weeks() > 1 {
                format!("{} weeks", off.whole_weeks())
            } else if off.whole_days() > 1 {
                format!("{} days", off.whole_days())
            } else if off.whole_hours() > 1 {
                format!("{} hours", off.whole_hours())
            } else {
                format!("{off}")
            }
        })
    })?;

    let off = off.unix_timestamp();

    // Find all tweets older than the provided offset, delete them,
    // and mark as deleted

    {
        use crate::schema::tweets::dsl::*;
        let conn = &mut conn;

        let t = tweets.filter(created_at.lt(&off));

        let found: Vec<MTweet> = t.load::<MTweet>(conn)?;

        {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            use hmac::{Hmac, Mac};
            use req::{
                blocking::{ClientBuilder, RequestBuilder},
                header,
                header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
                Method,
                Url,
            };
            use reqwest as req;
            use sha1::Sha1;
            use urlencoding::encode;

            type HmacSha1 = Hmac<Sha1>;

            /// Params is not encoded
            fn create_auth(
                keys: &Access,
                base_url: &str,
                method: Method,
                params: &[(String, String)],
            ) -> String {
                let mut rng = thread_rng();
                let auth = &[
                    //
                    ("oauth_consumer_key", &keys.api_key),
                    ("oauth_nonce", &Alphanumeric.sample_string(&mut rng, 32)),
                    ("oauth_signature_method", &"HMAC-SHA1".to_string()),
                    (
                        "oauth_timestamp",
                        &OffsetDateTime::now_utc().unix_timestamp().to_string(),
                    ),
                    ("oauth_token", &keys.access),
                    ("oauth_version", &"1.0".to_string()),
                ];
                // Percent encoded auth values
                let mut auth: Vec<_> = auth
                    .iter()
                    .map(|(k, v)| (encode(k).into_owned(), encode(v).into_owned()))
                    .collect();
                auth.sort_by(|a, b| a.0.cmp(&b.0));

                // Percent encoded Auth values used for generating the signature
                let mut sig = auth.clone();
                // Includes parameters
                sig.extend(
                    params
                        .iter()
                        .map(|(k, v)| (encode(k).into_owned(), encode(v).into_owned())),
                );
                // Has to be sorted
                sig.sort_by(|a, b| a.0.cmp(&b.0));

                // Parameter string
                // Sig is already percent encoded
                let mut param_string = String::new();
                for (k, v) in sig {
                    param_string.push_str(&k);
                    param_string.push('=');
                    param_string.push_str(&v);
                    param_string.push('&');
                }
                // Pop last &
                param_string.pop();

                // Signature base string
                let mut sig_base = String::new();
                sig_base.push_str(method.as_str());
                sig_base.push('&');
                sig_base.push_str(&encode(base_url));
                sig_base.push('&');
                sig_base.push_str(&encode(&param_string));

                // Sign key
                let mut sign_key = String::new();
                sign_key.push_str(&encode(&keys.api_secret));
                sign_key.push('&');
                sign_key.push_str(&encode(&keys.access_secret));

                // Sign it
                let mut mac: HmacSha1 = HmacSha1::new_from_slice(sign_key.as_bytes()).unwrap();
                mac.update(sig_base.as_bytes());
                let sig = mac.finalize().into_bytes();

                let sig = STANDARD.encode(sig);

                // Final auth header string
                // Everything is already percent encoded
                let mut auth_out = String::from("Oauth ");
                for (k, v) in auth.into_iter().chain(once((
                    "oauth_signature".to_string(),
                    encode(&sig).into_owned(),
                ))) {
                    auth_out.push_str(&k);
                    auth_out.push_str("=\"");
                    auth_out.push_str(&v);
                    auth_out.push('"');
                    auth_out.push_str(", ");
                }
                // Pop last comma and space
                auth_out.pop();
                auth_out.pop();

                auth_out
            }

            let mut client = ClientBuilder::new().build()?;

            // Lookup tweets in the DB and mark them as deleted if they don't exist
            let t = tweets.filter(deleted.eq(false)).load::<MTweet>(conn)?;

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

                let res = client
                    .post(TWEET_LOOKUP_URL)
                    .header(
                        AUTHORIZATION,
                        dbg!(create_auth(
                            &keys,
                            TWEET_LOOKUP_URL,
                            Method::POST,
                            &[
                                //
                                ("id".to_string(), ids.clone()),
                            ],
                        )),
                    )
                    .form(&[("id", ids.as_str())])
                    .send()?;
                dbg!(&res.status());
                dbg!(&res.text());
                break;
            }
        }

        // let delete = diesel::update(t).set(deleted.eq(true)).execute(conn)?;
        // dbg!(delete);
    };

    // let mut args = Args::parse();
    Ok(())
}
