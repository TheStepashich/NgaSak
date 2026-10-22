use anyhow::Result;
use std::process::Command;

pub fn run_command(cmd: &str, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new(cmd).args(args).output()?;
    Ok(output)
}

pub fn run_command_str(cmd: &str, args: &[&str]) -> Result<String> {
    let output = run_command(cmd, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
