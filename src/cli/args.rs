use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "qiuchenly",
    version,
    about = "秋城落叶应用商店命令行客户端",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct PageArgs {
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub page: u32,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,
}

#[derive(Args)]
pub struct TransferArgs {
    #[arg(long, default_value = "aria2c", help = "指定 aria2c 程序路径")]
    pub aria2c: PathBuf,
}

#[derive(Args)]
pub struct OutputArgs {
    #[arg(short = 'o', long, help = "指定下载目录，默认使用配置中的目录")]
    pub output_dir: Option<PathBuf>,
    #[command(flatten)]
    pub transfer: TransferArgs,
}

#[derive(Args)]
pub struct SelectionArgs {
    #[arg(short, long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct ChannelArgs {
    #[arg(long, default_value = "default", help = "指定商店渠道，默认官方渠道")]
    pub channel: String,
}

#[derive(Args)]
pub struct InstallArgs {
    pub app: String,
    #[arg(long)]
    pub version: Option<String>,
    #[arg(long, help = "自动选择版本时包含测试版")]
    pub beta: bool,
    #[arg(long, help = "下载并安装预破解包，默认使用官方包")]
    pub precracked: bool,
    #[arg(long, default_value = "/Applications", help = "指定应用安装目录")]
    pub appdir: PathBuf,
    #[arg(long, help = "安装后保留下载的文件")]
    pub keep_download: bool,
    #[arg(
        long,
        value_name = "LANGUAGE",
        help = "Adobe 安装语言（例如 en_US、zh_CN），默认 en_US"
    )]
    pub language: Option<String>,
    #[command(flatten)]
    pub transfer: TransferArgs,
    #[command(flatten)]
    pub channel: ChannelArgs,
}

#[derive(Subcommand)]
pub enum Command {
    /// 搜索商店应用
    Search {
        query: String,
        #[command(flatten)]
        page: PageArgs,
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 浏览应用分类
    Categories {
        key: Option<String>,
        #[arg(long, requires = "key")]
        query: Option<String>,
        #[command(flatten)]
        page: PageArgs,
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 查看应用详情
    Info {
        app: String,
        #[arg(long, help = "显示全部官方版本，默认显示前五个")]
        versions: bool,
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 列出已安装应用
    List {
        #[arg(long, help = "列出所有本地应用，默认只列出商店中的已安装应用")]
        all: bool,
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 查看已安装应用的商店更新
    Outdated {
        #[arg(value_name = "APP")]
        apps: Vec<String>,
        #[arg(long, help = "自动选择版本时包含测试版")]
        beta: bool,
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 查看热门下载
    Popular {
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 下载应用或输出下载链接
    Download {
        app: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long, help = "自动选择版本时包含测试版")]
        beta: bool,
        #[arg(long, help = "下载预破解包，默认下载官方包")]
        precracked: bool,
        #[arg(
            long,
            conflicts_with = "output_dir",
            help = "仅输出下载链接，不下载文件"
        )]
        url: bool,
        #[command(flatten)]
        output: OutputArgs,
        #[command(flatten)]
        channel: ChannelArgs,
    },
    /// 下载并安装应用
    Install(InstallArgs),
    /// 卸载应用并清理残留
    Uninstall {
        target: String,
        #[command(flatten)]
        selection: SelectionArgs,
    },
    /// 清理残留或孤儿文件
    Cleanup {
        bundle_id: Option<String>,
        #[command(flatten)]
        selection: SelectionArgs,
    },
    /// 查看卸载历史
    History {
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(1..))]
        limit: u64,
    },
    /// 登录商店账户
    Login {
        username: String,
        #[arg(long, help = "从标准输入读取密码，仅去除末尾换行符")]
        password_stdin: bool,
    },
    /// 查看当前账户
    Whoami,
    /// 退出登录
    Logout,
    /// 管理商店渠道
    Channel {
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// 管理卸载规则
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },
    /// 注入、还原、列出或更新支持的应用
    Inject {
        #[arg(
            required_unless_present_any = ["list", "update"],
            conflicts_with_all = ["list", "update"]
        )]
        /// 要注入或还原的应用，可指定多个
        app: Vec<String>,
        #[arg(long, requires = "app", conflicts_with_all = ["list", "update"])]
        restore: bool,
        #[arg(long, conflicts_with = "update")]
        list: bool,
        #[arg(long)]
        update: bool,
        #[command(flatten)]
        transfer: TransferArgs,
    },
}

#[derive(Subcommand)]
pub enum ChannelAction {
    /// 列出渠道
    List,
    /// 添加自定义渠道
    Add {
        name: String,
        #[arg(value_name = "BASE_URL")]
        base_url: String,
    },
    /// 移除自定义渠道
    Remove { name: String },
}

#[derive(Subcommand)]
pub enum RulesAction {
    /// 查看当前规则
    Show,
    /// 同步订阅规则
    Sync,
    /// 恢复内置规则
    Reset,
}
