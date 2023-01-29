-- Your SQL goes here
CREATE TABLE tweets (
    id INTEGER PRIMARY KEY NOT NULL,
    retweets INTEGER NOT NULL,
    likes INTEGER NOT NULL,
    created_at TEXT NOT NULL
) STRICT
