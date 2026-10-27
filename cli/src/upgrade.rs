/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use std::{
    env,
    error::Error,
    fs::{self, File},
    io::Write,
    path::PathBuf,
};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

impl Release {
    pub fn is_newer_than(&self, current: &str) -> Result<bool, Box<dyn Error>> {
        let parse = |tag: &str| -> Result<Version, Box<dyn Error>> {
            let version = tag
                .strip_prefix('v')
                .ok_or("Version tag must start with 'v'")?;
            Ok(Version::parse(version)?)
        };
        Ok(parse(&self.tag_name)?
            .cmp_precedence(&parse(current)?)
            .is_gt())
    }

    pub fn download_url(&self, platform: &str) -> Result<&str, Box<dyn Error>> {
        let name = format!("gaius-{platform}");
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
            .ok_or_else(|| format!("Release {} has no asset for {platform}", self.tag_name).into())
    }
}

struct StagedFile {
    path: PathBuf,
    file: Option<File>,
    committed: bool,
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub struct Upgrader {
    client: Client,
    repo: &'static str,
    curr: &'static str,
    plat: &'static str,
    exe: PathBuf,
}

impl Upgrader {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            client: Client::builder().user_agent("gaius-upgrade").build()?,
            repo: env!("UPGRADE_REPO"),
            curr: env!("TAG_VERSION"),
            plat: env!("PLATFORM"),
            exe: env::current_exe()?,
        })
    }

    pub fn new_from(
        repo: &'static str,
        curr: &'static str,
        plat: &'static str,
        exe: PathBuf,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            client: Client::builder().user_agent("gaius-upgrade").build()?,
            repo,
            curr,
            plat,
            exe,
        })
    }

    fn temp_file(&self) -> Result<StagedFile, Box<dyn Error>> {
        let dir = self
            .exe
            .parent()
            .ok_or("Executable has no parent directory")?;
        let basename = self.exe.file_name().ok_or("Unable to get filename")?;
        let permissions = fs::metadata(&self.exe)?.permissions();
        let mut name = basename.to_os_string();
        name.push(format!("_{}", Uuid::now_v7()));
        let path = dir.join(name);
        let file = File::options().write(true).create_new(true).open(&path)?;
        file.set_permissions(permissions)?;
        let staged = StagedFile {
            path,
            file: Some(file),
            committed: false,
        };
        Ok(staged)
    }

    async fn save_to(&self, release: &Release, output: &mut File) -> Result<(), Box<dyn Error>> {
        let url = release.download_url(self.plat)?;
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        while let Some(chunk) = response.chunk().await? {
            output.write_all(&chunk)?;
        }
        output.flush()?;
        output.sync_all()?;

        if output.metadata()?.len() == 0 {
            return Err("Downloaded executable is empty".into());
        }
        Ok(())
    }

    pub async fn run(&self) -> Result<(), Box<dyn Error>> {
        println!("Running upgrade...");
        let release: Release = self
            .client
            .get(self.repo)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if !release.is_newer_than(self.curr)? {
            println!("gaius is already up to date ({}).", self.curr);
            return Ok(());
        }

        println!("Found newer version: {}", release.tag_name);
        let mut staged = self.temp_file()?;

        println!("Downloading...");
        self.save_to(&release, staged.file.as_mut().unwrap())
            .await?;
        drop(staged.file.take());
        fs::rename(&staged.path, &self.exe)?;
        staged.committed = true;

        println!("Upgraded to: {}", release.tag_name);
        Ok(())
    }
}
