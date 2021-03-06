use std::{io, path::PathBuf};

use directories::ProjectDirs;
use libc::{self, _SC_PAGESIZE};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tracing::warn;

const QUALIFIER: &str = "io";
const ORGANIZATION: &str = "CasperLabs";
const APPLICATION: &str = "casperlabs-node";

const DEFAULT_MAX_BLOCK_STORE_SIZE: usize = 483_183_820_800; // 450 GiB
const DEFAULT_MAX_DEPLOY_STORE_SIZE: usize = 322_122_547_200; // 300 GiB

const DEFAULT_TEST_MAX_DB_SIZE: usize = 52_428_800; // 50 MiB

/// On-disk storage configuration.
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    /// The path to the folder where any files created or read by the storage component will exist.
    ///
    /// If the folder doesn't exist, it and any required parents will be created.
    ///
    /// Defaults to:
    /// * Linux: `$XDG_DATA_HOME/casperlabs-node` or `$HOME/.local/share/casperlabs-node`, e.g.
    ///   /home/alice/.local/share/casperlabs-node
    /// * macOS: `$HOME/Library/Application Support/io.CasperLabs.casperlabs-node`, e.g.
    ///   /Users/Alice/Library/Application Support/io.CasperLabs.casperlabs-node
    /// * Windows: `{FOLDERID_RoamingAppData}\CasperLabs\casperlabs-node\data` e.g.
    ///   C:\Users\Alice\AppData\Roaming\CasperLabs\casperlabs-node\data
    pub path: PathBuf,
    /// Sets the maximum size of the database to use for the block store.
    ///
    /// Defaults to 483,183,820,800 == 450 GiB.
    ///
    /// The size should be a multiple of the OS page size.
    pub max_block_store_size: usize,
    /// Sets the maximum size of the database to use for the deploy store.
    ///
    /// Defaults to 322,122,547,200 == 300 GiB.
    ///
    /// The size should be a multiple of the OS page size.
    pub max_deploy_store_size: usize,
}

impl Config {
    /// Returns a default `Config` suitable for tests, along with a `TempDir` which must be kept
    /// alive for the duration of the test since its destructor removes the dir from the filesystem.
    #[allow(unused)]
    pub(crate) fn default_for_tests() -> (Self, TempDir) {
        let tempdir = tempfile::tempdir().expect("should get tempdir");
        let path = tempdir.path().to_path_buf();

        let config = Config {
            path,
            max_block_store_size: DEFAULT_TEST_MAX_DB_SIZE,
            max_deploy_store_size: DEFAULT_TEST_MAX_DB_SIZE,
        };
        (config, tempdir)
    }

    /// Prints a warning if any max DB size is not a multiple of the OS page size.
    pub fn check_sizes(&self) {
        let page_size = get_page_size().unwrap_or(1);
        if self.max_block_store_size % page_size != 0 {
            warn!(
                "max block store DB size {} is not multiple of system page size {}",
                self.max_block_store_size, page_size
            );
        }
        if self.max_deploy_store_size % page_size != 0 {
            warn!(
                "max deploy store DB size {} is not multiple of system page size {}",
                self.max_deploy_store_size, page_size
            );
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let path = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .map(|project_dirs| project_dirs.data_dir().to_path_buf())
            .unwrap_or_else(|| {
                warn!("failed to get project dir - falling back to current dir");
                PathBuf::from(".")
            });

        let config = Config {
            path,
            max_block_store_size: DEFAULT_MAX_BLOCK_STORE_SIZE,
            max_deploy_store_size: DEFAULT_MAX_DEPLOY_STORE_SIZE,
        };

        config.check_sizes();
        config
    }
}

/// Returns OS page size
fn get_page_size() -> Result<usize, io::Error> {
    // https://www.gnu.org/software/libc/manual/html_node/Sysconf.html
    let value = unsafe { libc::sysconf(_SC_PAGESIZE) };

    if value < 0 {
        warn!("unable to get system page size");
        return Err(io::Error::last_os_error());
    }

    Ok(value as usize)
}
