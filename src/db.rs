use std::error;
use std::iter;
use std::path;


use reqwest;
use sqlite;


const SCHEMA_SQL: &str = "
    CREATE TABLE urls (
    	url TEXT NOT NULL UNIQUE,
    	path TEXT NOT NULL,
    	last_modified TEXT,
    	etag TEXT
    );
";


/// All the information we have about a given URL.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheRecord {
    /// The path to the cached response body on disk.
    pub path: String,
    /// The value of the Last-Modified header in the original response.
    pub last_modified: Option<reqwest::header::HttpDate>,
    /// The value of the Etag header in the original response.
    pub etag: Option<String>,
}


/// Represents the rows returned by a query.
struct Rows<'a>(sqlite::Cursor<'a>);


impl<'a> iter::Iterator for Rows<'a> {
    type Item = Vec<sqlite::Value>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
            .unwrap_or_else(|err| {
                warn!("Failed to get next row from SQLite: {}", err);
                None
            })
            .map(|values| values.to_vec())
    }
}

/// Represents the database that describes the contents of the cache.
pub struct CacheDB(sqlite::Connection);

impl CacheDB {
    /// Create a cache database in the given file.
    pub fn new<P: AsRef<path::Path>>(path: P)
        -> Result<CacheDB, Box<error::Error>>
    {
        // Package up the return value first, so we can use .query()
        // instead of wrangling sqlite directly.
        let res = CacheDB(sqlite::Connection::open(path)?);

        let rows: Vec<_> = res.query(
            "SELECT COUNT(*) FROM sqlite_master;",
            &[],
        )?.collect();
        if let sqlite::Value::Integer(0) = rows[0][0] {
            // No tables define in this DB, let's load our schema.
            res.0.execute(SCHEMA_SQL)?
        }

        Ok(res)
    }

    fn query<'a, T: AsRef<str>>(
        &'a self,
        query: T,
        params: &[sqlite::Value],
    ) -> sqlite::Result<Rows> {
        let mut cur = self.0.prepare(query)?.cursor();
        cur.bind(params)?;

        Ok(Rows(cur))
    }

    pub fn get(&self, mut url: reqwest::Url)
        -> Result<CacheRecord, Box<error::Error>>
    {
        url.set_fragment(None);

        let mut rows = self.query("
            SELECT path, last_modified, etag
            FROM urls
            WHERE url = ?1
            ",
            &[sqlite::Value::String(url.as_str().into())],
        )?;

        rows.next()
            .map_or(
                Err(format!("URL not found in cache: {:?}", url)),
                |x| Ok(x),
            )
            .map(|row| -> Result<CacheRecord, Box<error::Error>> {
                let mut cols = row.into_iter();

                let path = match cols.next().unwrap() {
                    sqlite::Value::String(s) => Ok(s),
                    other => Err(format!("Path had wrong type: {:?}", other)),
                }?;

                let last_modified = match cols.next().unwrap() {
                    sqlite::Value::String(s) => {
                        use std::str::FromStr;
                        Some(reqwest::header::HttpDate::from_str(&s)?)
                    },
                    sqlite::Value::Null => { None },
                    other => {
                        warn!(
                            "last_modified contained weird type: {:?}",
                            other,
                        );
                        None
                    },
                };

                let etag = match cols.next().unwrap() {
                    sqlite::Value::String(s) => { Some(s) },
                    sqlite::Value::Null => { None },
                    other => {
                        warn!(
                            "last_modified contained weird type: {:?}",
                            other,
                        );
                        None
                    },
                };

                Ok(CacheRecord{path, last_modified, etag})
            })?
    }
}


#[cfg(test)]
mod tests {
    extern crate tempdir;
    use reqwest;
    use sqlite;

    #[test]
    fn create_fresh_db() {
        let db = super::CacheDB::new(":memory:").unwrap();

        let rows: Vec<_> = db.query(
            "SELECT name FROM sqlite_master WHERE TYPE = ?1",
            &[sqlite::Value::String("table".into())],
        ).unwrap().collect();

        assert_eq!(rows, vec![vec![sqlite::Value::String("urls".into())]]);

    }

    #[test]
    fn reopen_existing_db() {
        let root = tempdir::TempDir::new("cachedb-test").unwrap().into_path();
        let db_path = root.join("cache.db");

        let db1 = super::CacheDB::new(&db_path).unwrap();
        let rows: Vec<_> = db1.query(
            "SELECT name FROM sqlite_master WHERE TYPE = ?1",
            &[sqlite::Value::String("table".into())],
        ).unwrap().collect();
        assert_eq!(rows, vec![vec![sqlite::Value::String("urls".into())]]);


        let db2 = super::CacheDB::new(&db_path).unwrap();
        let rows: Vec<_> = db2.query(
            "SELECT name FROM sqlite_master WHERE TYPE = ?1",
            &[sqlite::Value::String("table".into())],
        ).unwrap().collect();
        assert_eq!(rows, vec![vec![sqlite::Value::String("urls".into())]]);
    }

    #[test]
    fn open_bogus_db() {
        let res = super::CacheDB::new("does/not/exist");

        assert_eq!(res.is_err(), true);
    }

    #[test]
    fn get_from_empty_db() {
        let db = super::CacheDB::new(":memory:").unwrap();

        let err = db.get("http://example.com/".parse().unwrap()).unwrap_err();

        assert_eq!(
            err.description(),
            "URL not found in cache: \"http://example.com/\""
        );
    }

    #[test]
    fn get_unknown_url() {
        let db = super::CacheDB::new(":memory:").unwrap();

        db.0.execute("
            INSERT INTO urls
                ( url
                , path
                , last_modified
                , etag
                )
            VALUES
                ( 'http://example.com/one'
                , 'path/to/data'
                , NULL
                , NULL
                )
            ;
        ").unwrap();

        let err = db.get(
            "http://example.com/two".parse().unwrap()
        ).unwrap_err();

        assert_eq!(
            err.description(),
            "URL not found in cache: \"http://example.com/two\""
        );
    }

    #[test]
    fn get_known_url() {
        let db = super::CacheDB::new(":memory:").unwrap();

        db.0.execute("
            INSERT INTO urls
                ( url
                , path
                , last_modified
                , etag
                )
            VALUES
                ( 'http://example.com/'
                , 'path/to/data'
                , NULL
                , NULL
                )
            ;
        ").unwrap();

        let record = db.get(
            "http://example.com/".parse().unwrap()
        ).unwrap();

        assert_eq!(
            record,
            super::CacheRecord{
                path: "path/to/data".into(),
                last_modified: None,
                etag: None,
            }
        );
    }

    #[test]
    fn get_known_url_with_headers() {
        use std::str::FromStr;

        let db = super::CacheDB::new(":memory:").unwrap();
        db.0.execute("
            INSERT INTO urls
                ( url
                , path
                , last_modified
                , etag
                )
            VALUES
                ( 'http://example.com/'
                , 'path/to/data'
                , 'Thu, 01 Jan 1970 00:00:00 GMT'
                , 'some-crazy-text'
                )
            ;
        ").unwrap();

        let record = db.get(
            "http://example.com/".parse().unwrap()
        ).unwrap();

        assert_eq!(
            record,
            super::CacheRecord{
                path: "path/to/data".into(),
                last_modified: Some(reqwest::header::HttpDate::from_str(
                    "Thu, 01 Jan 1970 00:00:00 GMT"
                ).unwrap()),
                etag: Some("some-crazy-text".into()),
            }
        );
    }

    #[test]
    fn get_url_with_invalid_path() {

        let db = super::CacheDB::new(":memory:").unwrap();

        db.0.execute("
            INSERT INTO urls
                ( url
                , path
                , last_modified
                , etag
                )
            VALUES
                ( 'http://example.com/'
                , CAST('abc' AS BLOB)
                , NULL
                , NULL
                )
            ;
        ").unwrap();

        let err = db.get("http://example.com/".parse().unwrap()).unwrap_err();

        assert_eq!(
            err.description(),
            "Path had wrong type: Binary([97, 98, 99])"
        );
    }

    #[test]
    fn get_url_with_invalid_last_modified_and_etag() {

        let db = super::CacheDB::new(":memory:").unwrap();

        db.0.execute("
            INSERT INTO urls
                ( url
                , path
                , last_modified
                , etag
                )
            VALUES
                ( 'http://example.com/'
                , 'path/to/data'
                , CAST('abc' AS BLOB)
                , CAST('def' AS BLOB)
                )
            ;
        ").unwrap();

        let record = db.get("http://example.com/".parse().unwrap()).unwrap();

        assert_eq!(
            record,
            super::CacheRecord{
                path: "path/to/data".into(),
                // We expect TEXT or NULL; if we get a BLOB value we
                // treat it as NULL.
                last_modified: None,
                etag: None,
            }
        );
    }

    #[test]
    fn get_ignores_fragments() {
        let db = super::CacheDB::new(":memory:").unwrap();

        db.0.execute("
            INSERT INTO urls
                ( url
                , path
                , last_modified
                , etag
                )
            VALUES
                ( 'http://example.com/'
                , 'path/to/data'
                , NULL
                , NULL
                )
            ;
        ").unwrap();

        let record = db.get(
            "http://example.com/#top".parse().unwrap()
        ).unwrap();

        assert_eq!(
            record,
            super::CacheRecord{
                path: "path/to/data".into(),
                last_modified: None,
                etag: None,
            }
        );
    }
}
