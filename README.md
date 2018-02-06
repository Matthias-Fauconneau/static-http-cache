`static_http_cache`, a local disk cache for static HTTP resources
=================================================================

(insert links to documentation and crates.io)

TODO
----

  - proper error reporting
  - record usage counts and dates for entries in the cache, so we can
    automatically clean them up.
  - evaluate API against the [Rust API guidelines][rapig]
  - make sure each public type's interface is defined by a trait.
  - `Cache::get()` needs a callback to report download progress.
  - if `Cache::get()` updates the locally cached data, it should
    delete the file containing the stale data.
  - Add support for other caching-relevant headers, like Expires
    or Cache-Control.
  - Support "freshness", so we can sometimes answer from the cache
    without having to talk to the remote server at all.
    
[rapig]: https://rust-lang-nursery.github.io/api-guidelines/
