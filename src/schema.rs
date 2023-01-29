// @generated automatically by Diesel CLI.

diesel::table! {
    tweets (id) {
        id -> Integer,
        retweets -> Integer,
        likes -> Integer,
        created_at -> Text,
    }
}
