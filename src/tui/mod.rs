//! Terminal user interface built on ratatui.

use anyhow::Result;

use crate::paths::Paths;

pub fn run(_paths: Paths, _dry: bool) -> Result<i32> {
    anyhow::bail!("the interface is not built yet; use --run")
}
