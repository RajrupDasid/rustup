//! Installation and upgrade of both distribution-managed and local
//! toolchains

use crate::dist::component::{Components, Package, TarGzPackage, Transaction};
use crate::dist::dist;
use crate::dist::download::DownloadCfg;
use crate::dist::prefix::InstallPrefix;
use crate::dist::temp;
use crate::dist::Notification;
use crate::errors::Result;
use crate::notifications::Notification as RootNotification;
use crate::toolchain::{CustomToolchain, DistributableToolchain, Toolchain, UpdateStatus};
use crate::utils::utils;
use std::path::Path;

#[derive(Copy, Clone)]
pub enum InstallMethod<'a> {
    Copy(&'a Path, &'a CustomToolchain<'a>),
    Link(&'a Path, &'a CustomToolchain<'a>),
    Installer(&'a Path, &'a temp::Cfg, &'a CustomToolchain<'a>),
    // bool is whether to force an update
    Dist {
        desc: &'a dist::ToolchainDesc,
        profile: dist::Profile,
        update_hash: Option<&'a Path>,
        dl_cfg: DownloadCfg<'a>,
        // --force
        force_update: bool,
        // --allow-downgrade
        allow_downgrade: bool,
        // toolchain already exists
        exists: bool,
        // currently installed date
        old_date: Option<&'a str>,
        // Extra components to install from dist
        components: &'a [&'a str],
        // Extra targets to install from dist
        targets: &'a [&'a str],
        distributable: &'a DistributableToolchain<'a>,
    },
}

impl<'a> InstallMethod<'a> {
    // Install a toolchain
    pub fn install(&self, toolchain: &Toolchain<'a>) -> Result<UpdateStatus> {
        let previous_version = if toolchain.exists() {
            Some(toolchain.rustc_version())
        } else {
            None
        };
        if previous_version.is_some() {
            (toolchain.cfg().notify_handler)(RootNotification::UpdatingToolchain(
                &toolchain.name(),
            ));
        } else {
            (toolchain.cfg().notify_handler)(RootNotification::InstallingToolchain(
                &toolchain.name(),
            ));
        }
        (toolchain.cfg().notify_handler)(RootNotification::ToolchainDirectory(
            &toolchain.path(),
            &toolchain.name(),
        ));
        let updated = self.run(&toolchain.path(), &|n| {
            (toolchain.cfg().notify_handler)(n.into())
        })?;

        if !updated {
            (toolchain.cfg().notify_handler)(RootNotification::UpdateHashMatches);
        } else {
            (toolchain.cfg().notify_handler)(RootNotification::InstalledToolchain(
                &toolchain.name(),
            ));
        }

        let status = match (updated, previous_version) {
            (true, None) => UpdateStatus::Installed,
            (true, Some(v)) => UpdateStatus::Updated(v),
            (false, _) => UpdateStatus::Unchanged,
        };

        Ok(status)
    }

    pub fn run(self, path: &Path, notify_handler: &dyn Fn(Notification<'_>)) -> Result<bool> {
        if path.exists() {
            // Don't uninstall first for Dist method
            match self {
                InstallMethod::Dist { .. } | InstallMethod::Installer(..) => {}
                _ => {
                    uninstall(path, notify_handler)?;
                }
            }
        }

        match self {
            InstallMethod::Copy(src, ..) => {
                utils::copy_dir(src, path, notify_handler)?;
                Ok(true)
            }
            InstallMethod::Link(src, ..) => {
                utils::symlink_dir(src, &path, notify_handler)?;
                Ok(true)
            }
            InstallMethod::Installer(src, temp_cfg, ..) => {
                InstallMethod::tar_gz(src, path, &temp_cfg, notify_handler)?;
                Ok(true)
            }
            InstallMethod::Dist {
                desc,
                profile,
                update_hash,
                dl_cfg,
                force_update,
                allow_downgrade,
                exists,
                old_date,
                components,
                targets,
                ..
            } => {
                let prefix = &InstallPrefix::from(path.to_owned());
                let maybe_new_hash = dist::update_from_dist(
                    dl_cfg,
                    update_hash,
                    desc,
                    if exists { None } else { Some(profile) },
                    prefix,
                    force_update,
                    allow_downgrade,
                    old_date,
                    components,
                    targets,
                )?;

                if let Some(hash) = maybe_new_hash {
                    if let Some(hash_file) = update_hash {
                        utils::write_file("update hash", hash_file, &hash)?;
                    }

                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn tar_gz(
        src: &Path,
        path: &Path,
        temp_cfg: &temp::Cfg,
        notify_handler: &dyn Fn(Notification<'_>),
    ) -> Result<()> {
        notify_handler(Notification::Extracting(src, path));

        let prefix = InstallPrefix::from(path.to_owned());
        let installation = Components::open(prefix.clone())?;
        let notification_converter = |notification: crate::utils::Notification<'_>| {
            notify_handler(notification.into());
        };
        let reader = utils::FileReaderWithProgress::new_file(&src, &notification_converter)?;
        let package: &dyn Package =
            &TarGzPackage::new(reader, temp_cfg, Some(&notification_converter))?;

        let mut tx = Transaction::new(prefix, temp_cfg, notify_handler);

        for component in package.components() {
            tx = package.install(&installation, &component, None, tx)?;
        }

        tx.commit();

        Ok(())
    }
}

pub fn uninstall(path: &Path, notify_handler: &dyn Fn(Notification<'_>)) -> Result<()> {
    utils::remove_dir("install", path, notify_handler)
}
