// @generated automatically by Diesel CLI.

diesel::table! {
    tweets (id_str) {
        id_str -> Text,
        retweets -> Integer,
        likes -> Integer,
        created_at -> BigInt,
    }
}
