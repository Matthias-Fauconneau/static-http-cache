extern crate static_http_cache;
extern crate reqwest;

use std::env;
use std::error::Error;
use std::fs;
use std::io;


fn get_resource() -> Result<fs::File, Box<Error>>
{
    // Where shall we store our cache data?
    let cache_path = env::temp_dir().join("static_http_cache");

    // Create the directory to hold persistent cache data.
    fs::DirBuilder::new()
        .recursive(true)
        .create(&cache_path)?;

    // What URL should we download?
    let url = reqwest::Url::parse(
        "https://static.rust-lang.org/dist/channel-rust-stable.toml",
    )?;

    // Create the cache data structure we need on disk.
    let mut cache = static_http_cache::Cache::new(
        cache_path,
        reqwest::Client::new(),
    )?;

    // Actually retrieve the URL if needed.
    cache.get(url)
}


fn main() {
    let mut file = get_resource().expect("Could not download URL");

    // Squirt our downloaded file to stdout to prove we got it.
    let stdout = io::stdout();
    io::copy(&mut file, &mut stdout.lock()).expect("could not write to stdout");
}
