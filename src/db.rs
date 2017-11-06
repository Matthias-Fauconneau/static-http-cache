use std::error;
use std::iter;
use std::path;


use sqlite;


const SCHEMA_SQL: &str = "
    CREATE TABLE urls (
    	id INTEGER PRIMARY KEY,
    	url TEXT NOT NULL UNIQUE,
    	path TEXT NOT NULL,
    	last_modified TEXT,
    	etag TEXT
    );
";


const SETUP_SQL: &str = "
    PRAGMA foreign_keys = ON;
";


/// Represents the rows returned by a query.
struct Rows<'a>(sqlite::Cursor<'a>);


impl<'a> iter::Iterator for Rows<'a> {
    type Item = Vec<sqlite::Value>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().unwrap().map(|values| values.to_vec())
    }
}

/// Represents the database that describes the contents of the cache.
pub struct CacheDB(sqlite::Connection);

impl CacheDB {
    pub fn new<P: AsRef<path::Path>>(path: P)
        -> Result<CacheDB, Box<error::Error>>
    {
        let conn = sqlite::Connection::open(path)?;
        conn.execute(SETUP_SQL)?;

        let res = CacheDB(conn);

        let rows: Vec<_> = res.query(
            "SELECT COUNT(*) FROM sqlite_master;",
            &[],
        )?.collect();
        if let sqlite::Value::Integer(0) = rows[0][0] {
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
}


#[cfg(test)]
mod tests {
    extern crate tempdir;
    use sqlite;

    #[test]
    fn create_fresh_db() {
        let root = tempdir::TempDir::new("cachedb-test").unwrap().into_path();
        let db = super::CacheDB::new(root.join("cache.db")).unwrap();

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
}
