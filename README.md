`static_http_cache`, a local disk cache for static HTTP resources
=================================================================

  - [Documentation](https://docs.rs/static-http-cache/)
  - [Package](https://crates.io/crates/static-http-cache)
  - [Source code](https://gitlab.com/Screwtapello/static_http_cache)

TODO
----

  - proper error reporting
  - record usage counts and dates for entries in the cache, so we can
    automatically clean them up.
  - make sure each public type's interface is defined by a trait.
  - `Cache::get()` needs a callback to report download progress.
  - if `Cache::get()` updates the locally cached data, it should
    delete the file containing the stale data.
  - Add support for other caching-relevant headers, like Expires
    or Cache-Control.
  - Support "freshness", so we can sometimes answer from the cache
    without having to talk to the remote server at all.
