use anyhow::{bail, Context, Result};
use std::process::{Command, Output};

pub(crate) fn output(command: &mut Command, description: &str) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("运行 {description}"))?;
    if !output.status.success() {
        bail!(
            "{description}失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(output)
}

pub(crate) fn run(command: &mut Command, description: &str) -> Result<()> {
    output(command, description).map(|_| ())
}
