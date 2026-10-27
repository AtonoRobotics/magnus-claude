//! A fresh Postgres database per test, on the server named by `MAGNUS_TEST_PG`.

use tokio_postgres::{Client, NoTls};

pub fn server_url() -> String {
    std::env::var("MAGNUS_TEST_PG")
        .unwrap_or_else(|_| "postgres://postgres:magnus_dev@127.0.0.1:54340/postgres".into())
}

pub struct TestDb {
    pub name: String,
    pub kernel_role: String,
    pub kernel_password: String,
}

impl TestDb {
    pub async fn create() -> Self {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let name = format!("t_{id}");
        let c = connect(&server_url()).await;
        c.batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .expect("create test database");
        TestDb {
            name,
            kernel_role: format!("k_{id}"),
            kernel_password: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    fn with(&self, user: Option<(&str, &str)>) -> String {
        let mut u = url::Url::parse(&server_url()).expect("MAGNUS_TEST_PG is a URL");
        u.set_path(&format!("/{}", self.name));
        if let Some((name, pw)) = user {
            u.set_username(name).unwrap();
            u.set_password(Some(pw)).unwrap();
        }
        u.to_string()
    }

    /// Superuser on this test database.
    pub fn admin_url(&self) -> String {
        self.with(None)
    }

    /// The role the kernel runs as. `magnus migrate` creates it.
    pub fn kernel_url(&self) -> String {
        self.with(Some((&self.kernel_role, &self.kernel_password)))
    }

    pub async fn admin(&self) -> Client {
        connect(&self.admin_url()).await
    }

    pub async fn as_kernel(&self) -> Client {
        connect(&self.kernel_url()).await
    }

    /// Every user table outside the system schemas, as (schema, table).
    pub async fn tables(&self) -> Vec<(String, String)> {
        let c = self.admin().await;
        c.query(
            "SELECT table_schema, table_name FROM information_schema.tables
             WHERE table_type = 'BASE TABLE'
               AND table_schema NOT IN ('pg_catalog', 'information_schema')
             ORDER BY 1, 2",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect()
    }

    /// Every row of every user table, as text.
    pub async fn dump(&self) -> String {
        let c = self.admin().await;
        let mut out = String::new();
        for (s, t) in self.tables().await {
            let rows = c
                .query(&format!("SELECT row_to_json(x)::text FROM \"{s}\".\"{t}\" x"), &[])
                .await
                .unwrap();
            for r in rows {
                out.push_str(&format!("{s}.{t}: {}\n", r.get::<_, String>(0)));
            }
        }
        out
    }
}

pub async fn connect(url: &str) -> Client {
    let (client, conn) = tokio_postgres::connect(url, NoTls)
        .await
        .unwrap_or_else(|e| panic!("connect to {url}: {e}"));
    tokio::spawn(async move {
        let _ = conn.await;
    });
    client
}
