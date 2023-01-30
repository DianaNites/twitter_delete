//! Handles stuff related to interacting with the twitter API
use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    io::{stdout, Write},
    iter::once,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use clap::Parser;
use diesel::{prelude::*, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use hmac::{Hmac, Mac};
use rand::{
    distributions::{Alphanumeric, DistString},
    prelude::*,
    thread_rng,
};
use req::{
    blocking::{ClientBuilder, RequestBuilder},
    header,
    header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    Method,
    StatusCode,
    Url,
};
use reqwest as req;
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use sha1::Sha1;
use time::{
    format_description::{
        well_known::{iso8601::Config, Iso8601},
        FormatItem,
    },
    macros::format_description,
    Duration,
    OffsetDateTime,
    PrimitiveDateTime,
    UtcOffset,
};
use urlencoding::encode;

use crate::Access;

type HmacSha1 = Hmac<Sha1>;

/// Create twitter authentication headers
///
/// Params is not percent encoded
pub fn create_auth(
    keys: &Access,
    base_url: &str,
    method: Method,
    params: &[(String, String)],
) -> String {
    let mut rng = thread_rng();
    let auth = &[
        //
        ("oauth_consumer_key", &keys.api_key),
        ("oauth_nonce", &Alphanumeric.sample_string(&mut rng, 32)),
        ("oauth_signature_method", &"HMAC-SHA1".to_string()),
        (
            "oauth_timestamp",
            &OffsetDateTime::now_utc().unix_timestamp().to_string(),
        ),
        ("oauth_token", &keys.access),
        ("oauth_version", &"1.0".to_string()),
    ];
    // Percent encoded auth values
    let mut auth: Vec<_> = auth
        .iter()
        .map(|(k, v)| (encode(k).into_owned(), encode(v).into_owned()))
        .collect();
    auth.sort_by(|a, b| a.0.cmp(&b.0));

    // Percent encoded Auth values used for generating the signature
    let mut sig = auth.clone();
    // Includes parameters
    sig.extend(
        params
            .iter()
            .map(|(k, v)| (encode(k).into_owned(), encode(v).into_owned())),
    );
    // Has to be sorted
    sig.sort_by(|a, b| a.0.cmp(&b.0));

    // Parameter string
    // Sig is already percent encoded
    let mut param_string = String::new();
    for (k, v) in sig {
        param_string.push_str(&k);
        param_string.push('=');
        param_string.push_str(&v);
        param_string.push('&');
    }
    // Pop last &
    param_string.pop();

    // Signature base string
    let mut sig_base = String::new();
    sig_base.push_str(method.as_str());
    sig_base.push('&');
    sig_base.push_str(&encode(base_url));
    sig_base.push('&');
    sig_base.push_str(&encode(&param_string));

    // Sign key
    let mut sign_key = String::new();
    sign_key.push_str(&encode(&keys.api_secret));
    sign_key.push('&');
    sign_key.push_str(&encode(&keys.access_secret));

    // Sign it
    let mut mac: HmacSha1 = HmacSha1::new_from_slice(sign_key.as_bytes()).unwrap();
    mac.update(sig_base.as_bytes());
    let sig = mac.finalize().into_bytes();

    let sig = STANDARD.encode(sig);

    // Final auth header string
    // Everything is already percent encoded
    let mut auth_out = String::from("Oauth ");
    for (k, v) in auth.into_iter().chain(once((
        "oauth_signature".to_string(),
        encode(&sig).into_owned(),
    ))) {
        auth_out.push_str(&k);
        auth_out.push_str("=\"");
        auth_out.push_str(&v);
        auth_out.push('"');
        auth_out.push_str(", ");
    }
    // Pop last comma and space
    auth_out.pop();
    auth_out.pop();

    auth_out
}
