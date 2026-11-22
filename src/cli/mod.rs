mod args;
mod commands;

pub use args::Cli;
pub use commands::{account, catalog, channel, packages};

use crate::{inject, rules, uninstall};
use anyhow::Result;
use args::{ChannelAction, Command, RulesAction};

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Search {
                query,
                page,
                channel,
            } => catalog::search(&query, page, &channel.channel).await,
            Command::Categories {
                key,
                query,
                page,
                channel,
            } => catalog::categories(key, query, page, &channel.channel).await,
            Command::Info {
                app,
                versions,
                channel,
            } => catalog::info(&app, versions, &channel.channel).await,
            Command::List { all, channel } => catalog::list(all, &channel.channel).await,
            Command::Outdated {
                apps,
                beta,
                channel,
            } => catalog::outdated(apps, beta, &channel.channel).await,
            Command::Popular { channel } => catalog::popular(&channel.channel).await,
            Command::Download {
                app,
                version,
                beta,
                precracked,
                url,
                output,
                channel,
            } => {
                packages::download(
                    &app,
                    version.as_deref(),
                    beta,
                    precracked,
                    url,
                    output,
                    &channel.channel,
                )
                .await
            }
            Command::Install(args) => packages::install(&args).await,
            Command::Uninstall { target, selection } => {
                uninstall::uninstall_app(&target, selection.yes, selection.dry_run)
            }
            Command::Cleanup {
                bundle_id,
                selection,
            } => match bundle_id {
                Some(bundle_id) => {
                    uninstall::leftovers(&bundle_id, selection.yes, selection.dry_run)
                }
                None => uninstall::scan_orphans(selection.yes, selection.dry_run),
            },
            Command::History { offset, limit } => uninstall::show_history(offset, limit),
            Command::Login {
                username,
                password_stdin,
            } => account::login(&username, password_stdin).await,
            Command::Whoami => account::whoami().await,
            Command::Logout => account::logout(),
            Command::Channel { action } => match action {
                ChannelAction::List => channel::list(),
                ChannelAction::Add { name, base_url } => channel::add(&name, &base_url),
                ChannelAction::Remove { name } => channel::remove(&name),
            },
            Command::Rules { action } => match action {
                RulesAction::Show => rules::show(),
                RulesAction::Sync => rules::sync().await,
                RulesAction::Reset => rules::reset(),
            },
            Command::Inject {
                app,
                restore,
                list,
                update,
            } => {
                if update {
                    inject::update().await
                } else if list {
                    inject::list()
                } else {
                    let app =
                        app.expect("clap requires an app unless --list or --update is supplied");
                    if restore {
                        inject::restore_app(&app)
                    } else {
                        inject::inject_app(&app)
                    }
                }
            }
        }
    }
}
