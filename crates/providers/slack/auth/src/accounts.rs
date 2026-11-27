//! Non-secret account index (`$DATA/parla/accounts.json`, mode 0600).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use settings::WorkspaceKey;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub workspace: WorkspaceKey,
    pub name: String,
    pub url: Url,
    pub user_id: String,
    pub enterprise_id: Option<String>,
    /// Organisation name and workspace icon, refreshed from `team.info` on each start.
    pub enterprise_name: Option<String>,
    pub icon_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct WireAccount {
    workspace: String,
    name: String,
    url: Url,
    user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enterprise_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enterprise_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct WireIndex {
    v: u32,
    accounts: Vec<WireAccount>,
}

#[derive(Debug)]
pub struct AccountIndex {
    path: PathBuf,
    accounts: Vec<Account>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl AccountIndex {
    /// A missing file is an empty index; a corrupt one is an error.
    pub fn load(path: &Path) -> io::Result<AccountIndex> {
        let mut idx = AccountIndex {
            path: path.to_path_buf(),
            accounts: Vec::new(),
        };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(idx),
            Err(e) => return Err(e),
        };
        let wire: WireIndex =
            serde_json::from_str(&text).map_err(|_| invalid("account index is not valid"))?;
        if wire.v != 1 {
            return Err(invalid("unsupported account index version"));
        }
        for a in wire.accounts {
            idx.accounts.push(Account {
                workspace: a.workspace.parse().map_err(invalid)?,
                name: a.name,
                url: a.url,
                user_id: a.user_id,
                enterprise_id: a.enterprise_id,
                enterprise_name: a.enterprise_name,
                icon_url: a.icon_url,
            });
        }
        Ok(idx)
    }

    /// Loads the index, applies `f` and saves it, all behind one process-wide
    /// lock so concurrent writers never lose each other's changes.
    pub fn update(path: &Path, f: impl FnOnce(&mut AccountIndex)) -> io::Result<()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut idx = AccountIndex::load(path)?;
        f(&mut idx);
        idx.save()
    }

    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Replaces the account with the same workspace in place, else appends.
    pub fn upsert(&mut self, account: Account) {
        match self
            .accounts
            .iter_mut()
            .find(|a| a.workspace == account.workspace)
        {
            Some(slot) => *slot = account,
            None => self.accounts.push(account),
        }
    }

    pub fn remove(&mut self, ws: &WorkspaceKey) {
        self.accounts.retain(|a| &a.workspace != ws);
    }

    /// Atomic write (temp file + rename) with mode 0600 on unix.
    pub fn save(&self) -> io::Result<()> {
        let wire = WireIndex {
            v: 1,
            accounts: self
                .accounts
                .iter()
                .map(|a| WireAccount {
                    workspace: a.workspace.to_string(),
                    name: a.name.clone(),
                    url: a.url.clone(),
                    user_id: a.user_id.clone(),
                    enterprise_id: a.enterprise_id.clone(),
                    enterprise_name: a.enterprise_name.clone(),
                    icon_url: a.icon_url.clone(),
                })
                .collect(),
        };
        let text = serde_json::to_string_pretty(&wire).map_err(|e| invalid(e.to_string()))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &self.path)
    }
}
