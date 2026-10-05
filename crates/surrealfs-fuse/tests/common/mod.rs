use std::process::{Child, Command, Stdio};
use std::time::Duration;
use surrealfs_core::{ConnectOptions, SurrealFs};
use tokio::sync::OnceCell;

static SERVER_URL: OnceCell<String> = OnceCell::const_new();
static mut SERVER_GUARD: Option<Child> = None;

pub async fn get_server_url() -> &'static str {
    SERVER_URL
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("SURREALFS_TEST_URL") {
                return url;
            }

            let port = 18450;
            let url = format!("ws://127.0.0.1:{}", port);

            let child = Command::new("surreal")
                .args([
                    "start",
                    "--user",
                    "root",
                    "--pass",
                    "root",
                    "--bind",
                    &format!("127.0.0.1:{}", port),
                    "memory",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("Failed to spawn surreal test server. Is surreal installed?");

            unsafe {
                SERVER_GUARD = Some(child);
            }

            let mut ready = false;
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let opts = ConnectOptions {
                    url: url.clone(),
                    user: Some("root".to_string()),
                    pass: Some("root".to_string()),
                    ns: "test".to_string(),
                    db: "test".to_string(),
                    caller: None,
                };
                if SurrealFs::connect(opts).await.is_ok() {
                    ready = true;
                    break;
                }
            }

            if !ready {
                panic!("SurrealDB server failed to become ready on {}", url);
            }

            url
        })
        .await
        .as_str()
}

pub async fn create_test_fs() -> SurrealFs {
    let url = get_server_url().await;
    let ns = format!("test_{}", rand_id());
    let db = "test".to_string();

    let opts = ConnectOptions {
        url: url.to_string(),
        user: Some("root".to_string()),
        pass: Some("root".to_string()),
        ns,
        db,
        caller: None,
    };

    let fs = SurrealFs::connect(opts)
        .await
        .expect("Failed to connect to test server");

    // Apply schema
    let schema_file = include_str!("../../../../surrealfs/schema/file.surql");
    let schema_auth = include_str!("../../../../surrealfs/schema/record_auth.surql");

    fs.client()
        .query(schema_file)
        .await
        .expect("Failed to apply file.surql schema");
    fs.client()
        .query(schema_auth)
        .await
        .expect("Failed to apply record_auth.surql schema");

    fs
}

fn rand_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos)
}
