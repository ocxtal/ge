use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Git;

pub struct GrepArgs<'a> {
    pub pattern: &'a str,
    pub context: Option<usize>,
    pub before: Option<usize>,
    pub after: Option<usize>,
}

impl Git {
    pub fn new() -> Result<Self> {
        // check the availability of the git command
        let output = Command::new("git")
            .args(&["--version"])
            .output()
            .context("\"git\" command not found.")?;
        assert!(output.status.success());

        Ok(Git)
    }

    pub fn grep(&self, args: &GrepArgs) -> Result<String> {
        // compose arguments
        let mut grep_args = vec![
            "grep".to_string(),
            "--color=never".to_string(),
            "--line-number".to_string(),
        ];
        if let Some(c) = args.context {
            grep_args.push(format!("--context={}", c));
        }
        if let Some(b) = args.before {
            grep_args.push(format!("--before={}", b));
        }
        if let Some(a) = args.after {
            grep_args.push(format!("--after={}", a));
        }
        grep_args.push(args.pattern.to_string());

        // run git-grep then parse the output as a utf-8 string
        let output = Command::new("git")
            .args(&grep_args)
            .output()
            .context("failed to get output of \"git grep\". aborting.")?;
        let output = String::from_utf8(output.stdout).context(
            "failed to interpret the output of \"git grep\" as a UTF-8 string. aborting.",
        )?;

        Ok(output)
    }

    pub fn apply(&self, patch: &str) -> Result<()> {
        let mut apply = Command::new("git")
            .args(&["apply", "--allow-empty", "--unidiff-zero", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("failed to run \"git apply\". aborting.")?;

        // we expect it's dropped after use (it sends EOF)
        {
            let mut stdin = apply.stdin.take().unwrap();
            stdin.write_all(patch.as_bytes()).unwrap();
        }

        // make sure patch was successful
        let code = apply
            .wait()
            .context("\"git apply\" unexpectedly exited. aborting.")?;
        if !code.success() {
            return Err(anyhow!(
                "\"git apply\" returned an error{}. aborting.",
                code
            ));
        }

        Ok(())
    }
}
