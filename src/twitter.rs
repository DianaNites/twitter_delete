//! Handles stuff related to interacting with the twitter API
use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    io::{stdout, Write},
    iter::once,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::Parser;
use diesel::{prelude::*, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use hmac::{Hmac, Mac};
use rand::{
    distributions::{Alphanumeric, DistString},
    prelude::*,
    thread_rng,
};
use req::{
    blocking::{ClientBuilder, RequestBuilder},
    header,
    header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    Method,
    StatusCode,
    Url,
};
use reqwest as req;
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use sha1::Sha1;
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
use urlencoding::encode;

use crate::Access;

type HmacSha1 = Hmac<Sha1>;

/// Twitter tweet object. Internal, useless.
#[derive(Debug, Deserialize)]
struct TweetObj {
    tweet: Tweet,
}

/// A Tweet in the twitter archive.
///
/// NOTE: This is ***different*** than what would be returned by
/// the twitter API.
#[derive(Debug, Deserialize)]
pub struct Tweet {
    /// Tweet ID
    ///
    /// Currently 19 characters long, a 64-bit number.
    pub id_str: String,

    /// Number of retweets
    #[serde(rename = "retweet_count")]
    pub retweets: String,

    /// Number of likes
    #[serde(rename = "favorite_count")]
    pub likes: String,

    /// Time of tweet
    ///
    /// See [`TWITTER_DATE`]
    pub created_at: String,
}

#[cfg(no)]
impl Tweet {
    pub fn id(&self) -> &str {
        self.id_str.as_ref()
    }

    pub fn retweets(&self) -> &str {
        self.retweet_count.as_ref()
    }

    pub fn likes(&self) -> &str {
        self.like_count.as_ref()
    }

    pub fn created_at(&self) -> &str {
        self.created_at.as_ref()
    }
}

/// Create twitter authentication headers
///
/// Params is not percent encoded
pub fn create_auth(
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

/// Collect tweets from the twitter archive. Returns ALL found tweets.
///
/// `path` is the path to the archive, and tweets are expected to exist at
/// `data/tweets.js` and `data/tweets-partN.js`.
///
/// There is a limit of 99 `tweets-partN.js` files
pub fn collect_tweets(path: &Path) -> Result<Vec<Tweet>> {
    let mut files = Vec::with_capacity(99);
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
        if files.len() > 99 {
            return Err(anyhow!("Too many tweet files, can not handle more than 99"));
        }
        files.push(file.path());
    }

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
    }

    Ok(out)
}
