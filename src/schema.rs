// @generated automatically by Diesel CLI.

diesel::table! {
    accounts (id_str) {
        id_str -> Text,
        user_name -> Text,
        display_name -> Text,
    }
}

diesel::table! {
    tweets (id_str) {
        id_str -> Text,
        retweets -> Integer,
        likes -> Integer,
        created_at -> BigInt,
        deleted -> Bool,
        checked -> Bool,
        account_id -> Text,
    }
}

diesel::joinable!(tweets -> accounts (account_id));

diesel::allow_tables_to_appear_in_same_query!(accounts, tweets,);
