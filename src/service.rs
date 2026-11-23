#[cfg(not(target_os = "linux"))]
pub fn install(_origin: &str) -> anyhow::Result<()> {
    anyhow::bail!("service install requires a Linux host with systemd")
}

#[cfg(target_os = "linux")]
pub use linux::install;

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs::{self, File, OpenOptions, Permissions},
        io::{self, Cursor, Read},
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        path::Path,
        process::Command,
    };

    use crate::auth::new_secret;
    use anyhow::{Context, Result, bail, ensure};
    use runlever::config::{Catalog, normalize_origin};

    const EXAMPLE: &str = include_str!("../examples/config.toml");
    const SCRIPT: &str = include_str!("../examples/compose.sh");
    const UNIT: &str = include_str!("../packaging/runlever.service");

    fn command(program: &str, args: &[&str]) -> Result<()> {
        let status = Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("start {program}"))?;
        ensure!(status.success(), "{program} {args:?} exited with {status}");
        Ok(())
    }

    fn has_entry(database: &str) -> Result<bool> {
        let result = Command::new("getent")
            .args([database, "runlever"])
            .output()
            .context("look up service account with getent")?;
        match result.status.code() {
            Some(0) => Ok(true),
            Some(2) => Ok(false),
            _ => bail!(
                "getent {database} failed ({}): {}",
                result.status,
                String::from_utf8_lossy(&result.stderr)
            ),
        }
    }

    /// Publish complete files on the destination filesystem. Configuration and the
    /// example script are created once; upgrades replace the binary and unit atomically.
    fn publish(
        destination: &Path,
        content: &mut impl Read,
        mode: u32,
        group: Option<&str>,
        replace: bool,
    ) -> Result<bool> {
        if !replace && destination.try_exists()? {
            return Ok(false);
        }
        let parent = destination.parent().context("missing install directory")?;
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(".runlever-install-{}", new_secret()?));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        let result = (|| -> Result<bool> {
            io::copy(content, &mut file)?;
            if let Some(group) = group {
                let status = Command::new("chgrp").arg(group).arg(&temp).status()?;
                ensure!(
                    status.success(),
                    "set group of {}: {status}",
                    destination.display()
                );
            }
            file.set_permissions(Permissions::from_mode(mode))?;
            file.sync_all()?;
            drop(file);
            if replace {
                fs::rename(&temp, destination)?;
            } else {
                match fs::hard_link(&temp, destination) {
                    Ok(()) => (),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(true)
        })();
        if let Err(err) = fs::remove_file(&temp)
            && err.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path=%temp.display(), error=?err, "clean install temporary file");
        }
        result.with_context(|| format!("install {}", destination.display()))
    }

    pub fn install(origin: &str) -> Result<()> {
        let user = Command::new("id")
            .arg("-u")
            .output()
            .context("identify installer user")?;
        ensure!(
            user.status.success() && user.stdout.trim_ascii() == b"0",
            "run service install as root (sudo)"
        );
        let dir = Path::new("/etc/runlever");
        fs::create_dir_all(dir)?;
        let guard = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.join(".install.lock"))?;
        guard
            .try_lock()
            .context("another installation is in progress")?;
        if !has_entry("group")? {
            command("groupadd", &["--system", "runlever"])?;
        }
        if !has_entry("passwd")? {
            command(
                "useradd",
                &[
                    "--system",
                    "--gid",
                    "runlever",
                    "--home-dir",
                    "/var/lib/runlever",
                    "--no-create-home",
                    "--shell",
                    "/usr/sbin/nologin",
                    "runlever",
                ],
            )?;
        }
        command("chgrp", &["runlever", "/etc/runlever"])?;
        fs::set_permissions(dir, Permissions::from_mode(0o750))?;
        let config = dir.join("config.toml");
        if !config.try_exists()? {
            let origin = normalize_origin(origin).context("invalid --origin")?;
            let admin = new_secret()?;
            let example = EXAMPLE
                .replace("https://localhost", &origin)
                .replace("RUNLEVER_GENERATED_ADMIN_TOKEN", &admin)
                .replace("RUNLEVER_GENERATED_FORGEJO_TOKEN", &new_secret()?);
            if publish(
                &config,
                &mut Cursor::new(example),
                0o640,
                Some("runlever"),
                false,
            )? {
                println!("首次安装的网页登录 Token：{admin}");
            }
        } else {
            // A manually provisioned configuration may initially be readable only by root.
            command("chown", &["root:runlever", "/etc/runlever/config.toml"])?;
            fs::set_permissions(&config, Permissions::from_mode(0o640))?;
        }
        Catalog::load(&config)?
            .check_secrets()
            .context("validate installed configuration")?;
        let scripts = Path::new("/usr/local/lib/runlever");
        if !scripts.try_exists()? {
            fs::create_dir_all(scripts)?;
            fs::set_permissions(scripts, Permissions::from_mode(0o755))?;
        }
        publish(
            Path::new("/usr/local/lib/runlever/compose.sh"),
            &mut Cursor::new(SCRIPT),
            0o755,
            None,
            false,
        )?;
        let destination = Path::new("/usr/local/bin/runlever");
        let current = std::env::current_exe()?;
        publish(destination, &mut File::open(current)?, 0o755, None, true)?;
        publish(
            Path::new("/etc/systemd/system/runlever.service"),
            &mut Cursor::new(UNIT),
            0o644,
            None,
            true,
        )?;
        command("systemctl", &["daemon-reload"])?;
        command("systemctl", &["enable", "runlever.service"])?;
        command("systemctl", &["restart", "runlever.service"])?;
        println!(
            "Runlever 已安装。配置在 {}，服务状态可用 systemctl status runlever 查看。",
            config.display()
        );
        Ok(())
    }
}
