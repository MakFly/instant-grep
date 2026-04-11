//! `ig docker <subcmd> [args]` — filtered Docker command wrapper.
//!
//! Prepends "docker" to the arguments and delegates to the generic
//! filtered runner, which applies matching Docker filters automatically.

use anyhow::Result;

/// Run a Docker command with automatic output filtering.
pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig docker <subcommand> [args...]");
    }

    let mut full_args = vec!["docker".to_string()];
    full_args.extend(args.iter().cloned());

    crate::cmds::run::run(&full_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_returns_error() {
        let result = run(&[]);
        assert!(result.is_err());
    }
}
