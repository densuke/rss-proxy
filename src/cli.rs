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
    /// フィード固有の Processor 連鎖の管理
    #[command(subcommand)]
    Proc(ProcCmd),
    /// 全フィード共通の Processor 連鎖の管理
    #[command(subcommand)]
    Global(GlobalCmd),
}

#[derive(Subcommand)]
pub enum FeedCmd {
    /// フィードを登録する
    Add {
        url: String,
        /// 配信 URL に使う識別子。省略すると推測されにくい乱数から作る
        #[arg(long)]
        slug: Option<String>,
        /// 表示名。省略すると上流フィードのタイトルを使う
        #[arg(long)]
        label: Option<String>,
        #[arg(long, default_value_t = 900)]
        interval: i64,
    },
    /// 登録済みフィードを一覧する
    List,
    /// フィードの詳細と Processor 連鎖を表示する
    Show { slug: String },
    /// 識別子・表示名・巡回間隔を変更する
    Set {
        slug: String,
        /// 新しい識別子。変更すると購読中の配信 URL が変わる
        #[arg(long)]
        new_slug: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        interval: Option<i64>,
    },
    /// フィードを削除する
    Rm { slug: String },
}

#[derive(Subcommand)]
pub enum ProcCmd {
    /// 利用可能な Processor 種別を一覧する
    List,
    /// 連鎖の内容を表示する
    Show { feed: String },
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

/// 全フィード共通の連鎖。フィード固有の連鎖より先に走る。
#[derive(Subcommand)]
pub enum GlobalCmd {
    /// 連鎖の内容を表示する
    Show,
    /// Processor を追加する
    Attach {
        kind: String,
        #[arg(long)]
        params: Option<String>,
        #[arg(long)]
        at: Option<usize>,
    },
    /// 指定位置の Processor を外す
    Detach { position: usize },
    /// Processor の順序を入れ替える
    Move { from: usize, to: usize },
}

pub fn run(store: &Store, command: Command) -> Result<String> {
    match command {
        Command::Feed(cmd) => feed(store, cmd),
        Command::Proc(cmd) => processor(store, cmd),
        Command::Global(cmd) => global(store, cmd),
    }
}

fn feed(store: &Store, cmd: FeedCmd) -> Result<String> {
    match cmd {
        FeedCmd::Add {
            url,
            slug,
            label,
            interval,
        } => {
            if let Some(slug) = &slug {
                check_slug(slug)?;
            }
            let slug = store
                .add_feed(&NewFeed {
                    slug,
                    label,
                    url,
                    interval_secs: interval,
                })
                .context("フィードを登録できません")?;
            Ok(format!("登録しました: {slug}"))
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
                    format!(
                        "{}\t{}\t{}\t{}s\t{state}",
                        f.slug,
                        f.label.as_deref().or(f.title.as_deref()).unwrap_or("-"),
                        f.url,
                        f.interval_secs
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        FeedCmd::Show { slug } => {
            let f = find(store, &slug)?;
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
                "slug: {}\nlabel: {}\nurl: {}\ntitle: {}\ninterval: {}s\n\
                 processors:\n{chain}",
                f.slug,
                f.label.as_deref().unwrap_or("-"),
                f.url,
                f.title.as_deref().unwrap_or("-"),
                f.interval_secs,
            ))
        }
        FeedCmd::Set {
            slug,
            new_slug,
            label,
            interval,
        } => {
            let f = find(store, &slug)?;
            if let Some(new) = &new_slug {
                check_slug(new)?;
            }
            if new_slug.is_some() || label.is_some() {
                let next = new_slug.clone().unwrap_or_else(|| f.slug.clone());
                let label = label.or(f.label.clone());
                store
                    .rename(f.id, &next, label.as_deref())
                    .with_context(|| format!("{next} は既に使われています"))?;
                // 表示名は配信する title にも使う。作り直させる
                reschedule(store, f.id)?;
            }
            if let Some(interval) = interval {
                store.set_interval(f.id, interval)?;
            }
            Ok(format!("更新しました: {}", new_slug.unwrap_or(slug)))
        }
        FeedCmd::Rm { slug } => {
            if !store.remove_feed(&slug)? {
                bail!("フィード {slug} は登録されていません");
            }
            Ok(format!("削除しました: {slug}"))
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

        ProcCmd::Show { feed } => show(store, Chain::of(store, &feed)?),
        ProcCmd::Attach {
            feed,
            kind,
            params,
            at,
        } => attach(store, Chain::of(store, &feed)?, kind, params, at),
        ProcCmd::Detach { feed, position } => detach(store, Chain::of(store, &feed)?, position),
        ProcCmd::Move { feed, from, to } => reorder(store, Chain::of(store, &feed)?, from, to),
    }
}

fn global(store: &Store, cmd: GlobalCmd) -> Result<String> {
    match cmd {
        GlobalCmd::Show => show(store, Chain::Global),
        GlobalCmd::Attach { kind, params, at } => attach(store, Chain::Global, kind, params, at),
        GlobalCmd::Detach { position } => detach(store, Chain::Global, position),
        GlobalCmd::Move { from, to } => reorder(store, Chain::Global, from, to),
    }
}

fn show(store: &Store, target: Chain) -> Result<String> {
    let chain = target.read(store)?;
    if chain.is_empty() {
        return Ok("(なし)".into());
    }
    Ok(chain
        .iter()
        .enumerate()
        .map(|(i, (kind, params))| format!("{i}. {kind} {params}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn attach(
    store: &Store,
    target: Chain,
    kind: String,
    params: Option<String>,
    at: Option<usize>,
) -> Result<String> {
    // 保存前に組み立てて検証する。壊れた設定を DB に残さない。
    // 省略されたパラメータは既定値で埋めて保存する
    let built = proc::build(&kind, params.as_deref().unwrap_or("{}"))?;

    let mut chain = target.read(store)?;
    let at = at.unwrap_or(chain.len()).min(chain.len());
    chain.insert(at, (kind.clone(), built.params()));
    target.write(store, &chain)?;
    Ok(format!("{target} の {at} 番目に {kind} を追加しました"))
}

fn detach(store: &Store, target: Chain, position: usize) -> Result<String> {
    let mut chain = target.read(store)?;
    if position >= chain.len() {
        bail!("{target} に位置 {position} の Processor はありません");
    }
    let (kind, _) = chain.remove(position);
    target.write(store, &chain)?;
    Ok(format!("{target} から {kind} を外しました"))
}

fn reorder(store: &Store, target: Chain, from: usize, to: usize) -> Result<String> {
    let mut chain = target.read(store)?;
    if from >= chain.len() || to >= chain.len() {
        bail!("{target} の位置指定が範囲外です (0..{})", chain.len());
    }
    let item = chain.remove(from);
    chain.insert(to, item);
    target.write(store, &chain)?;
    Ok(format!("{target} の {from} を {to} へ移動しました"))
}

/// 操作対象の連鎖。全フィード共通か、特定フィードか。
enum Chain {
    Global,
    Feed { id: i64, slug: String },
}

impl Chain {
    fn of(store: &Store, slug: &str) -> Result<Self> {
        let f = find(store, slug)?;
        Ok(Self::Feed {
            id: f.id,
            slug: slug.to_string(),
        })
    }

    fn read(&self, store: &Store) -> Result<Vec<crate::store::ProcessorSpec>> {
        Ok(match self {
            Self::Global => store.global_processors()?,
            Self::Feed { id, .. } => store.processors(*id)?,
        })
    }

    fn write(&self, store: &Store, chain: &[crate::store::ProcessorSpec]) -> Result<()> {
        match self {
            Self::Global => {
                store.set_global_processors(chain)?;
                // 全フィードに効くので、全部を作り直しの対象にする
                for feed in store.list_feeds()? {
                    reschedule(store, feed.id)?;
                }
            }
            Self::Feed { id, .. } => {
                store.set_processors(*id, chain)?;
                reschedule(store, *id)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Global => write!(f, "全フィード共通"),
            Self::Feed { slug, .. } => write!(f, "{slug}"),
        }
    }
}

fn find(store: &Store, slug: &str) -> Result<crate::store::Feed> {
    store
        .feed_by_slug(slug)?
        .with_context(|| format!("フィード {slug} は登録されていません"))
}

/// 設定を変えたあと、次の巡回で配信内容を作り直させる。
///
/// 検証子を残したままだと 304 で処理がスキップされ、次回取得時刻が先のままだと
/// 最大で巡回間隔ぶん反映が遅れる。どちらも「設定したのに効いていない」ように見える。
fn reschedule(store: &Store, id: i64) -> Result<()> {
    store.clear_validators(id)?;
    store.set_next_fetch_at(id, 0)?;
    Ok(())
}

/// 配信 URL に載せられる形式かどうか。
fn check_slug(slug: &str) -> Result<()> {
    if !crate::slug::is_valid(slug) {
        bail!(
            "識別子 {slug} は使えません。英数字と - _ のみ、3〜64 文字にしてください \
             (URL エンコードなしでパスに載せるため)"
        );
    }
    Ok(())
}
