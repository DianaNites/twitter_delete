#![allow(
    unused_imports,
    dead_code,
    unreachable_code,
    unused_mut,
    unused_variables,
    clippy::let_and_return
)]
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::from_str;

/// Parse tweets from your twitter archive
#[derive(Parser, Debug)]
struct Args {
    /// Path to your twitter archive
    ///
    /// This is the folder with "Your archive.html" in it.
    path: PathBuf,
}

fn collect_tweets() -> Vec<()> {
    let mut out = Vec::new();
    //
    out
}

fn main() -> Result<()> {
    let mut args = Args::parse();

    Ok(())
}
