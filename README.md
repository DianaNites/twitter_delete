# Twitter Delete

A simple tool to process a twitter archive of its tweets, and then delete them,
subject to some basic criteria

This tool REQUIRES Twitter API v1.1 access.
This tool exclusively uses legacy v1.1 endpoints, NOT the new v2 endpoints.

This is because those are the only ones I have access to.

## Usage

The first thing you need to do is *import* your twitter archive,
as so

```shell
twitter_delete import PATH/TO/ARCHIVE/DIR
```

This will look for the various `data/tweet.js` and `data/tweet-partN.js` files,
importing them all into a sqlite database at `$HOME/.config/twitter_delete/tweets.db`.

After importing them, it will check every tweet for whether it's
already been deleted from Twitter or not.
This is done in batches of `100` using the [v1.1 Lookup API][1],
to not waste work and the rate limit on already deleted tweets.

After this is done, you can delete tweets subject to some simple filters

**WARNING**: These filters are based ***ONLY*** on data in your twitter archive.
The latest information from twitter is **NOT** checked.

If run without `--older-than`, this command will fail.
If you want to potentially delete **ALL** tweets,
you **MUST** pass `--older-than 0`

To delete all tweets older than 30 days,
unless they have at least 2 likes **and** 1 retweet,
and excluding the tweets ID `123456` or `7890`, this is the command.

If you would like an *or* check, you will have to run this command multiple times

```shell
twitter_delete delete \
    --older-than 30 \
    --unless-likes 2 \
    --unless-retweets 1 \
    --exclude 123456,7890
```

This is done using the [v1.1 Destroy API][2]. This can only be done one at a time.

[1]: <https://developer.twitter.com/en/docs/twitter-api/v1/tweets/post-and-engage/api-reference/get-statuses-lookup>
[2]: <https://developer.twitter.com/en/docs/twitter-api/v1/tweets/post-and-engage/api-reference/post-statuses-destroy-id>
