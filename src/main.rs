#![allow(
    unused_imports,
    dead_code,
    unreachable_code,
    unused_mut,
    unused_variables,
    clippy::let_and_return,
    clippy::never_loop
)]
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use time::{format_description::FormatItem, macros::format_description};

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

fn collect_tweets(path: &Path) -> Result<Vec<()>> {
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
        for tweet in data {
            let tw = tweet.tweet;
            dbg!(&tw);
            let time = time::PrimitiveDateTime::parse(&tw.created_at, TWITTER_DATE)?;
            dbg!(&time);
            break;
        }
        break;
    }

    Ok(out)
}

fn main() -> Result<()> {
    let keys: Access = from_str(ACCESS)?;
    let tweets = collect_tweets(&keys.test_path)?;
    // let mut args = Args::parse();

    Ok(())
}
