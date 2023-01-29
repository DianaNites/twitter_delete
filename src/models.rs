use diesel::prelude::*;

#[derive(Debug, Queryable)]
pub struct Tweet {
    /// Tweet ID
    pub id_str: i64,

    /// Number of retweets
    pub retweet: i32,

    /// Number of likes
    pub likes: i32,

    /// Time of tweet, in ISO-8601.
    pub created_at: String,
}
