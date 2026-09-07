use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rss_proxy::{auth, cli, fetch, scheduler, store::Store, web};

#[derive(Parser)]
#[command(
    name = "rss-proxy",
    version,
    about = "RSS/Atom フィードを整形して再配信する"
)]
struct Args {
    /// SQLite ファイルのパス
    #[arg(long, default_value = "rss-proxy.db", global = true)]
    db: PathBuf,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// 巡回と配信を開始する
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
    },
    /// 処理の適用結果を確認する
    Preview { name: String },
    /// 指定フィードを今すぐ取得して処理する
    Fetch { name: String },
    /// 管理画面のパスワードハッシュを生成する
    HashPassword,
    #[command(flatten)]
    Manage(cli::Command),
}

fn main() -> Result<()> {
    let args = Args::parse();
    // ハッシュ生成は DB を必要としない
    if matches!(args.command, Cmd::HashPassword) {
        return hash_password();
    }

    let store =
        Store::open(&args.db).with_context(|| format!("DB を開けません: {}", args.db.display()))?;

    match args.command {
        Cmd::Serve { listen } => serve(args.db, listen),
        Cmd::Preview { name } => {
            println!("{}", preview(&store, &name)?);
            Ok(())
        }
        Cmd::Fetch { name } => fetch_now(&store, &name),
        Cmd::HashPassword => unreachable!("DB を開く前に処理済み"),
        Cmd::Manage(cmd) => {
            println!("{}", cli::run(&store, cmd)?);
            Ok(())
        }
    }
}

/// 保存済みの配信内容を表示する。Processor の効果はこれで確認する。
fn preview(store: &Store, name: &str) -> Result<String> {
    store
        .output(name)?
        .with_context(|| format!("{name} はまだ一度も取得されていません"))
}

/// 管理画面のパスワードハッシュを作る。設定は環境変数で渡す。
fn hash_password() -> Result<()> {
    let password = read_password()?;
    if password.is_empty() {
        anyhow::bail!("パスワードが空です");
    }

    let hash = auth::hash_password(&password).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{}=admin", auth::USER_ENV);
    println!("{}='{hash}'", auth::HASH_ENV);
    eprintln!("\nこの 2 行を systemd の EnvironmentFile などに設定してください。");
    Ok(())
}

/// 端末なら伏せ字で 2 回入力させる。パイプ経由なら 1 行読む (スクリプト用)。
fn read_password() -> Result<String> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\n', '\r']).to_string());
    }

    let password = rpassword::prompt_password("管理画面のパスワード: ")?;
    if password != rpassword::prompt_password("もう一度入力: ")? {
        anyhow::bail!("入力が一致しません");
    }
    Ok(password)
}

/// サーバーを介さずその場で取得する。停止中でも実行できる。
fn fetch_now(store: &Store, name: &str) -> Result<()> {
    let feed = store
        .feed_by_name(name)?
        .with_context(|| format!("フィード {name} は登録されていません"))?;

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(scheduler::refresh(store, &fetch::client(), &feed))?;

    println!("取得しました: {name}");
    Ok(())
}

#[tokio::main]
async fn serve(db: PathBuf, listen: String) -> Result<()> {
    // 巡回と配信で別々の接続を開く。WAL により同時アクセスできる
    let server = Store::open(&db)?;

    // 巡回は専用スレッドで動かす。SQLite の接続は Sync ではないため、
    // マルチスレッドランタイム上のタスクとしては spawn できない
    std::thread::spawn(move || {
        let store = match Store::open(&db) {
            Ok(store) => store,
            Err(e) => {
                eprintln!("scheduler: DB を開けません: {e}");
                return;
            }
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("巡回用ランタイムの構築に失敗")
            .block_on(scheduler::run_loop(store, fetch::client()));
    });

    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("{listen} を listen できません"))?;
    eprintln!("listening on {listen}");
    // 接続元アドレスを middleware から参照できるようにする
    axum::serve(
        listener,
        web::app_with_auth(server, auth::Admin::from_env())
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
