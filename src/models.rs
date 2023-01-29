use diesel::{
    backend::Backend,
    prelude::*,
    serialize::{self, IsNull, Output, ToSql},
    sql_types::Text,
    sqlite::Sqlite,
    AsExpression,
    FromSqlRow,
    IntoSql,
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

#[derive(Debug, Queryable, AsExpression)]
#[diesel(sql_type = Text)]
pub struct DateTime(pub PrimitiveDateTime);

impl From<DateTime> for String {
    fn from(val: DateTime) -> Self {
        val.0.format(&DB_DATE).unwrap()
    }
}

impl ToSql<Text, Sqlite> for DateTime
where
    String: ToSql<Text, Sqlite>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let s = self.0.format(&DB_DATE)?;
        out.set_value(s);
        Ok(IsNull::No)
    }
}
