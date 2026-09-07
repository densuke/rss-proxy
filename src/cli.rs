//! CLI。サーバー API を経由せず SQLite を直接読み書きする。
//! サーバーが停止していても設定を編集できる。

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::proc;
use crate::store::{NewFeed, Store};

#[derive(Subcommand)]
pub enum Command {
    /// フィードの管理
    #[command(subcommand)]
    Feed(FeedCmd),
    /// Processor 連鎖の管理
    #[command(subcommand)]
    Proc(ProcCmd),
}

#[derive(Subcommand)]
pub enum FeedCmd {
    /// フィードを登録する
    Add {
        name: String,
        url: String,
        #[arg(long, default_value_t = 900)]
        interval: i64,
    },
    /// 登録済みフィードを一覧する
    List,
    /// フィードの詳細と Processor 連鎖を表示する
    Show { name: String },
    /// フィードを削除する
    Rm { name: String },
}

#[derive(Subcommand)]
pub enum ProcCmd {
    /// 利用可能な Processor 種別を一覧する
    List,
    /// Processor を連鎖に追加する
    Attach {
        feed: String,
        kind: String,
        /// JSON。省略すると既定値
        #[arg(long)]
        params: Option<String>,
        /// 挿入位置。省略すると末尾
        #[arg(long)]
        at: Option<usize>,
    },
    /// 指定位置の Processor を外す
    Detach { feed: String, position: usize },
    /// Processor の順序を入れ替える
    Move {
        feed: String,
        from: usize,
        to: usize,
    },
}

pub fn run(store: &Store, command: Command) -> Result<String> {
    match command {
        Command::Feed(cmd) => feed(store, cmd),
        Command::Proc(cmd) => processor(store, cmd),
    }
}

fn feed(store: &Store, cmd: FeedCmd) -> Result<String> {
    match cmd {
        FeedCmd::Add {
            name,
            url,
            interval,
        } => {
            store
                .add_feed(&NewFeed {
                    name: name.clone(),
                    url,
                    interval_secs: interval,
                })
                .with_context(|| format!("フィード {name} を登録できません"))?;
            Ok(format!("登録しました: {name}"))
        }
        FeedCmd::List => {
            let feeds = store.list_feeds()?;
            if feeds.is_empty() {
                return Ok("登録されているフィードはありません".into());
            }
            Ok(feeds
                .iter()
                .map(|f| {
                    let state = match (&f.last_error, f.last_success_at) {
                        (Some(e), _) => format!("失敗{}回: {e}", f.fail_count),
                        (None, Some(_)) => "正常".into(),
                        (None, None) => "未取得".into(),
                    };
                    format!("{}\t{}\t{}s\t{state}", f.name, f.url, f.interval_secs)
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        FeedCmd::Show { name } => {
            let f = find(store, &name)?;
            let chain = store
                .processors(f.id)?
                .iter()
                .enumerate()
                .map(|(i, (kind, params))| format!("  {i}. {kind} {params}"))
                .collect::<Vec<_>>();
            let chain = if chain.is_empty() {
                "  (なし)".to_string()
            } else {
                chain.join("\n")
            };
            Ok(format!(
                "name: {}\nurl: {}\ntitle: {}\ninterval: {}s\nenabled: {}\nprocessors:\n{chain}",
                f.name,
                f.url,
                f.title.as_deref().unwrap_or("-"),
                f.interval_secs,
                f.enabled,
            ))
        }
        FeedCmd::Rm { name } => {
            if !store.remove_feed(&name)? {
                bail!("フィード {name} は登録されていません");
            }
            Ok(format!("削除しました: {name}"))
        }
    }
}

fn processor(store: &Store, cmd: ProcCmd) -> Result<String> {
    match cmd {
        ProcCmd::List => Ok(proc::catalog()
            .iter()
            .map(|info| {
                let params = info
                    .params
                    .iter()
                    .map(|p| format!("\n      {} (既定 {}) {}", p.name, p.default, p.description))
                    .collect::<String>();
                format!("{}\n  {}{params}", info.kind, info.summary)
            })
            .collect::<Vec<_>>()
            .join("\n\n")),
        ProcCmd::Attach {
            feed,
            kind,
            params,
            at,
        } => {
            // 保存前に組み立てて検証する。壊れた設定を DB に残さない。
            // 省略されたパラメータは既定値で埋めて保存する
            let built = proc::build(&kind, params.as_deref().unwrap_or("{}"))?;

            let f = find(store, &feed)?;
            let mut chain = store.processors(f.id)?;
            let at = at.unwrap_or(chain.len()).min(chain.len());
            chain.insert(at, (kind.clone(), built.params()));
            store.set_processors(f.id, &chain)?;
            Ok(format!("{feed} の {at} 番目に {kind} を追加しました"))
        }
        ProcCmd::Detach { feed, position } => {
            let f = find(store, &feed)?;
            let mut chain = store.processors(f.id)?;
            if position >= chain.len() {
                bail!("{feed} に位置 {position} の Processor はありません");
            }
            let (kind, _) = chain.remove(position);
            store.set_processors(f.id, &chain)?;
            Ok(format!("{feed} から {kind} を外しました"))
        }
        ProcCmd::Move { feed, from, to } => {
            let f = find(store, &feed)?;
            let mut chain = store.processors(f.id)?;
            if from >= chain.len() || to >= chain.len() {
                bail!("{feed} の位置指定が範囲外です (0..{})", chain.len());
            }
            let item = chain.remove(from);
            chain.insert(to, item);
            store.set_processors(f.id, &chain)?;
            Ok(format!("{feed} の {from} を {to} へ移動しました"))
        }
    }
}

fn find(store: &Store, name: &str) -> Result<crate::store::Feed> {
    store
        .feed_by_name(name)?
        .with_context(|| format!("フィード {name} は登録されていません"))
}
