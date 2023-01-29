use anyhow::anyhow;
use diesel::{
    backend::Backend,
    deserialize::FromSql,
    prelude::*,
    serialize::{self, IsNull, Output, ToSql},
    sql_types::{BigInt, Integer, Text},
    sqlite::Sqlite,
    AsExpression,
    FromSqlRow,
    IntoSql,
};
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::{schema::tweets, DB_DATE};

#[derive(Queryable, Insertable)]
#[diesel(table_name = tweets)]
pub struct Tweet {
    /// Tweet ID
    pub id_str: String,

    /// Number of retweets
    pub retweets: i32,

    /// Number of likes
    pub likes: i32,

    /// Time of tweet, UTC unix time
    pub created_at: i64,
}

impl std::fmt::Debug for Tweet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut f = f.debug_struct("Tweet");
        f.field("id_str", &self.id_str)
            .field("retweets", &self.retweets)
            .field("likes", &self.likes);
        if let Ok(t) = OffsetDateTime::from_unix_timestamp(self.created_at)
            .map_err(|e| anyhow!(e))
            .and_then(|f| f.format(super::TWITTER_DATE).map_err(|e| anyhow!(e)))
        {
            f.field("created_at", &t);
        } else {
            f.field("created_at", &self.created_at);
        }
        f.finish()
    }
}
