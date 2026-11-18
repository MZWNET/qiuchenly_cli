mod args;
mod commands;

pub use args::Cli;
pub use commands::{account, catalog, packages};

use crate::{inject, rules, uninstall};
use anyhow::Result;
use args::{Command, CommunityAction, RulesAction};

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Search { query, page } => catalog::search(&query, page).await,
            Command::Categories { key, query, page } => catalog::categories(key, query, page).await,
            Command::Info { app, versions } => catalog::info(&app, versions).await,
            Command::List { all } => catalog::list(all).await,
            Command::Outdated { apps } => catalog::outdated(apps).await,
            Command::Popular => catalog::popular().await,
            Command::StoreVersion { current_version } => {
                catalog::store_version(&current_version).await
            }
            Command::Download {
                app,
                version,
                url,
                output,
            } => packages::download(&app, version.as_deref(), url, output).await,
            Command::Install {
                app,
                version,
                appdir,
                keep_download,
                transfer,
            } => {
                packages::install(&app, version.as_deref(), &appdir, keep_download, transfer).await
            }
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
            Command::Community { action } => match action {
                CommunityAction::List { app } => packages::community_list(&app).await,
                CommunityAction::Download {
                    app,
                    release,
                    arch,
                    attachment,
                    output,
                } => {
                    packages::community_download(
                        &app,
                        release.as_deref(),
                        arch.as_deref(),
                        attachment,
                        output,
                    )
                    .await
                }
            },
            Command::Rules { action } => match action {
                RulesAction::Show => rules::show(),
                RulesAction::Sync => rules::sync().await,
                RulesAction::Reset => rules::reset(),
            },
            Command::Inject { app, restore, list } => {
                if list {
                    inject::list()
                } else {
                    let app = app.expect("clap requires an app unless --list is supplied");
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
