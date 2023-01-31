//! Handles stuff related to interacting with the twitter API
use std::{
    collections::HashMap,
    fs,
    iter::once,
    path::Path,
    thread::sleep,
    time::Duration as StdDuration,
};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use rand::{
    distributions::{Alphanumeric, DistString},
    thread_rng,
};
use req::{
    blocking::{Client, RequestBuilder, Response},
    header::AUTHORIZATION,
    Method,
    StatusCode,
};
use reqwest as req;
use serde::Deserialize;
use serde_json::from_str;
use sha1::Sha1;
use time::OffsetDateTime;
use urlencoding::encode;

use crate::Access;

type HmacSha1 = Hmac<Sha1>;

/// Lookup 100 tweet IDs at a time
///
/// https://developer.twitter.com/en/docs/twitter-api/v1/tweets/post-and-engage/api-reference/get-statuses-lookup
pub const TWEET_LOOKUP_URL: &str = "https://api.twitter.com/1.1/statuses/lookup.json";

/// Delete a tweet
///
/// Ends in `{id}.json`
///
/// https://developer.twitter.com/en/docs/twitter-api/v1/tweets/post-and-engage/api-reference/post-statuses-destroy-id
pub const TWEET_DESTROY_URL: &str = "https://api.twitter.com/1.1/statuses/destroy/";

/// Indicates the rate limit response from the server
#[derive(Debug, Clone, Copy)]
pub enum RateLimit {
    /// Represents that the rate limit resets at this time in the future
    ///
    /// UTC Unix time
    ///
    /// Note that this is an absolute value in the future, not a number of
    /// seconds to wait
    Until(u64),

    /// Represents that it is unknown when the rate limit resets.
    ///
    /// This is handled by defaulting to 15 minutes
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LookupResp {
    pub id: HashMap<String, Option<LookupTweet>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LookupTweet {
    /// Tweet ID
    pub id_str: String,

    /// Number of retweets
    pub retweet_count: u64,

    /// Number of likes
    #[serde(rename = "favorite_count")]
    #[serde(default)]
    pub like_count: u64,

    /// Time of tweet
    ///
    /// See [`TWITTER_DATE`]
    pub created_at: String,
}

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

/// Lookup `tweets` on twitter.
///
/// `tweets` is a list of tweet IDs to lookup
///
/// Note that this twitter API can only look up tweets in batches of up to 100,
/// so this will call `on_chunk` for each successfully processed chunk.
///
/// Calls `on_limit` whenever a rate limit is hit.
pub fn lookup_tweets<'a, OnLimit, OnChunk>(
    client: &Client,
    keys: &Access,
    tweets: impl Iterator<Item = &'a str>,
    on_limit: OnLimit,
    on_chunk: OnChunk,
) -> Result<()>
where
    OnLimit: FnMut(RateLimit, &Response) -> Result<()>,
    OnChunk: FnMut(Response) -> Result<()>,
{
    let mut on_limit = on_limit;
    let mut on_chunk = on_chunk;
    let mut tweets = tweets;
    let tweets = tweets.by_ref();

    loop {
        let ids = tweets.take(100).collect::<Vec<&str>>().join(",");
        if ids.is_empty() {
            break;
        }
        let params = &[
            //
            ("id", ids.as_str()),
            ("map", "true"),
        ];

        let req = client
            .post(TWEET_LOOKUP_URL)
            .header(
                AUTHORIZATION,
                create_auth(
                    keys,
                    TWEET_LOOKUP_URL,
                    Method::POST,
                    &params.map(|f| (f.0.to_owned(), f.1.to_owned())),
                ),
            )
            .form(params);
        let res = rate_limit(&req, &mut on_limit)?;
        on_chunk(res)?;
    }

    Ok(())
}

/// Handles rate limiting with the Twitter API
///
/// Sends request `req`, and if a rate limit error is returned,
/// waits either until the time specified by twitter, or 15 minutes,
/// and then repeats the request.
///
/// Before waiting, calls `on_limit`. If this returns an error, it is returned.
fn rate_limit<F: FnMut(RateLimit, &Response) -> Result<()>>(
    req: &RequestBuilder,
    on_limit: F,
) -> Result<Response> {
    let mut on_limit = on_limit;

    let res = loop {
        let req = req
            .try_clone()
            .expect("BUG: Failed to clone RequestBuilder");

        let res = req.send()?;
        if res.status().is_success() {
            break res;
        } else if res.status() == StatusCode::TOO_MANY_REQUESTS {
            if let Some(r) = res
                .headers()
                .get("x-rate-limit-reset")
                .map(|f| f.to_str())
                .transpose()?
            {
                let secs: u64 = r.parse()?;
                on_limit(RateLimit::Until(secs), &res)?;

                // Default to 15 minutes
                let secs = (secs as i64)
                    .checked_sub(OffsetDateTime::now_utc().unix_timestamp())
                    .unwrap_or(60 * 15);
                sleep(StdDuration::from_secs(secs as u64));
            } else {
                on_limit(RateLimit::Unknown, &res)?;

                // Try waiting 15 minutes if there was no reset
                // header
                sleep(StdDuration::from_secs(60 * 15));
            }
        } else if res.status().is_server_error() {
            // Wait a minute and retry on transient server errors
            eprintln!(
                "Encountered transient HTTP error {}, waiting one minute\nData: {}",
                res.status(),
                res.text()?
            );
            sleep(StdDuration::from_secs(60));
        } else if res.status().is_client_error() {
            return Err(anyhow!(
                "Encountered HTTP error {}\nData: {}",
                res.status(),
                res.text()?
            ));
        }
    };

    Ok(res)
}
