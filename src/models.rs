use anyhow::anyhow;
use diesel::prelude::*;
use time::OffsetDateTime;

use crate::{schema::tweets, twitter::TWITTER_DATE};

#[derive(Queryable, Insertable, Clone)]
#[diesel(table_name = tweets)]
pub struct Tweet {
    /// Tweet ID. Primary key, Unique.
    pub id_str: String,

    /// Number of retweets
    pub retweets: i32,

    /// Number of likes
    pub likes: i32,

    /// Time of tweet, UTC unix time
    pub created_at: i64,

    /// Whether the tweet has been deleted
    pub deleted: bool,

    /// Whether the tweet has already been checked for existence
    pub checked: bool,
}

impl Tweet {
    pub fn new(id_str: String, retweets: i32, likes: i32, created_at: i64) -> Self {
        Self {
            id_str,
            retweets,
            likes,
            created_at,
            deleted: false,
            checked: false,
        }
    }
}

impl std::fmt::Debug for Tweet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut f = f.debug_struct("Tweet");
        f.field("id_str", &self.id_str)
            .field("retweets", &self.retweets)
            .field("likes", &self.likes);
        if let Ok(t) = OffsetDateTime::from_unix_timestamp(self.created_at)
            .map_err(|e| anyhow!(e))
            .and_then(|f| f.format(TWITTER_DATE).map_err(|e| anyhow!(e)))
        {
            f.field("created_at", &t);
        } else {
            f.field("created_at", &self.created_at);
        }
        f.finish()
    }
}
