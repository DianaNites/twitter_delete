-- Your SQL goes here
CREATE TABLE tweets (
    id_str TEXT PRIMARY KEY NOT NULL,
    retweets INTEGER NOT NULL,
    likes INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
