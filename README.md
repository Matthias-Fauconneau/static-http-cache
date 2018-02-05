`static_http_cache`, a local disk cache for static HTTP resources
=================================================================

This library maintains a cache of HTTP resources
in a local directory you specify.
Whenever you ask it for the contents of a URL,
it will re-use a previously-downloaded copy
if the resource has not changed on the server.
Otherwise,
it will download the new version and use that instead.

`static_http_cache` uses the [Reqwest][rq] crate for HTTP operations,
so it should properly handle HTTPS negotiation
and use the operating-system-provided certificate store.

Currently,
`static_http_cache` only uses the `Last-Modified` and `ETag` HTTP headers
to determine when its cached data is out of date.
Therefore,
it's not suitable for general-purpose HTTP caching;
it's best suited for static content like Amazon S3 data,
or Apache or nginx serving up a filesystem directory.

[rq]: https://crates.io/crates/reqwest

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
