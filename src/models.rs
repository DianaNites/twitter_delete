use diesel::{
    backend::Backend,
    prelude::*,
    serialize::{self, Output, ToSql},
    sql_types,
    AsExpression,
    FromSqlRow,
};
use time::PrimitiveDateTime;

use crate::{schema::tweets, DB_DATE};

#[derive(Debug, Queryable, Insertable)]
#[diesel(table_name = tweets)]
pub struct Tweet {
    /// Tweet ID
    pub id_str: String,

    /// Number of retweets
    pub retweets: i32,

    /// Number of likes
    pub likes: i32,

    /// Time of tweet, in a subset of ISO-8601, see [`DB_DATE`]
    #[diesel(serialize_as = String)]
    pub created_at: DateTime,
}

#[derive(Debug)]
pub struct DateTime(pub PrimitiveDateTime);

impl From<DateTime> for String {
    fn from(val: DateTime) -> Self {
        val.0.format(&DB_DATE).unwrap()
    }
}
