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
use time::PrimitiveDateTime;

use crate::{schema::tweets, DB_DATE};

// #[derive(Debug, FromSqlRow, AsExpression)]
// #[diesel(sql_type = BigInt)]
// pub struct DieselCrapFix(i64);

// impl Into<i64> for DieselCrapFix {
//     fn into(self) -> i64 {
//         self.0
//     }
// }

// impl ToSql<BigInt, Sqlite> for DieselCrapFix
// where
//     i64: ToSql<BigInt, Sqlite>,
// {
//     fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) ->
// serialize::Result {         self.0.to_sql(out)
//     }
// }

// impl Queryable<BigInt, Sqlite> for DieselCrapFix
// where
//     i64: FromSql<BigInt, Sqlite>,
// {
//     type Row = i64;

//     fn build(row: Self::Row) -> diesel::deserialize::Result<Self> {
//         Ok(row)
//     }
// }

#[derive(Debug, Queryable, Insertable)]
#[diesel(table_name = tweets)]
pub struct Tweet {
    /// Tweet ID
    pub id_str: String,

    /// Number of retweets
    pub retweets: i32,

    /// Number of likes
    pub likes: i32,

    /// Time of tweet, UTC unix time
    // FIXME: Diesel hates us and has no way to use `i64` and `Insertable` with STRICT tables
    // So this is a 32-bit timestamp for now so i can stop fighting this shit
    // #[diesel(serialize_as = DieselCrapFix)]
    // #[diesel(deserialize_as = DieselCrapFix)]
    pub created_at: i32,
}
