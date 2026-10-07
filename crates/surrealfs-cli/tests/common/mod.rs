use std::net::TcpListener;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;
use surrealdb::engine::remote::ws::Ws;
use surrealdb::Surreal;
use surrealfs_core::{ConnectOptions, SurrealFs};
use tokio::sync::OnceCell;

static SERVER_URL: OnceCell<String> = OnceCell::const_new();
static SERVER_PROC: Mutex<Option<Child>> = Mutex::new(None);

fn get_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind free port");
    listener.local_addr().unwrap().port()
}

pub async fn get_server_url() -> &'static str {
    SERVER_URL
        .get_or_init(|| async {
            if let Ok(u) = std::env::var("SURREALFS_TEST_URL") {
                return u;
            }

            let port = get_free_port();
            let proc = Command::new("surreal")
                .args([
                    "start",
                    "--allow-all",
                    "-u",
                    "root",
                    "-p",
                    "root",
                    "--bind",
                    &format!("127.0.0.1:{}", port),
                    "memory",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("Failed to start surreal process. Ensure `surreal` binary is on PATH.");

            {
                let mut lock = SERVER_PROC.lock().unwrap();
                *lock = Some(proc);
            }

            let url = format!("127.0.0.1:{}/rpc", port);

            // Wait until surrealdb accepts WS connections
            let mut ready = false;
            for _ in 0..100 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if Surreal::new::<Ws>(&url).await.is_ok() {
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

pub async fn create_test_fs() -> (SurrealFs, String, String) {
    let url = get_server_url().await;
    let ns = format!("test_cli_{}", rand_id());
    let db = "test".to_string();

    let opts = ConnectOptions {
        url: url.to_string(),
        user: Some("root".to_string()),
        pass: Some("root".to_string()),
        ns: ns.clone(),
        db: db.clone(),
        caller: None,
    };

    let mut fs = None;
    for attempt in 0..5 {
        match SurrealFs::connect(opts.clone()).await {
            Ok(f) => {
                fs = Some(f);
                break;
            }
            Err(_) if attempt < 4 => {
                tokio::time::sleep(Duration::from_millis(50 * (attempt + 1))).await;
            }
            Err(e) => panic!("Failed to connect to test server: {:?}", e),
        }
    }
    let fs = fs.unwrap();

    // Apply schema
    let schema_file = include_str!("../../../../surrealfs/schema/file.surql");
    let schema_auth = include_str!("../../../../surrealfs/schema/record_auth.surql");

    for attempt in 0..5 {
        match fs.client().query(schema_file).await {
            Ok(_) => break,
            Err(e) if attempt < 4 && e.to_string().contains("conflict") => {
                tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await;
            }
            Err(e) => panic!("Failed to apply file.surql schema: {:?}", e),
        }
    }

    for attempt in 0..5 {
        match fs.client().query(schema_auth).await {
            Ok(_) => break,
            Err(e) if attempt < 4 && e.to_string().contains("conflict") => {
                tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await;
            }
            Err(e) => panic!("Failed to apply record_auth.surql schema: {:?}", e),
        }
    }

    (fs, ns, db)
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn rand_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("{:x}_{}", nanos, count)
}
