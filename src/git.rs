use anyhow::{anyhow, Context, Result};
use clap::{ArgEnum, Parser};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Git;

#[derive(Copy, Clone, Debug, ArgEnum)]
enum GrepMode {
    Fixed,
    Extended,
    Basic,
    Pcre,
}

#[derive(Debug, Parser)]
pub struct GrepOptions {
    #[clap(
        arg_enum,
        short = 'M',
        long = "mode",
        default_value = "basic",
        help = "Regex mode"
    )]
    mode: GrepMode,

    #[clap(
        short = 'C',
        long,
        name = "N",
        help = "Include <N> additional lines before and after matches"
    )]
    context: Option<usize>,

    #[clap(
        short = 'B',
        long = "before-context",
        name = "B",
        help = "Include <B> additional lines before matches"
    )]
    before: Option<usize>,

    #[clap(
        short = 'A',
        long = "after-context",
        name = "A",
        help = "Include <A> additional lines after matches"
    )]
    after: Option<usize>,

    #[clap(
        short = 'W',
        long = "funciton-context",
        help = "Extend match to the entire function"
    )]
    function: bool,

    #[clap(short = 'v', long = "invert-match", help = "Invert matches")]
    invert: bool,

    #[clap(short = 'i', long = "ignore-case", help = "Case-insensitive search")]
    ignore_case: bool,

    #[clap(short = 'w', long = "word-regexp", help = "Match at word boundaries")]
    word_boundary: bool,

    #[clap(
        long = "max-depth",
        help = "Maximum directory depth to search [default: inf]"
    )]
    max_depth: Option<usize>,

    #[clap(
        short = 'y',
        long,
        help = "Files to search (in pathspec; multiple allowed)"
    )]
    only: Vec<String>,

    #[clap(
        short = 'x',
        long,
        help = "Files to exclude in search (in pathspec; multiple allowed)"
    )]
    exclude: Vec<String>,
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

    fn expand_options(&self, opts: &GrepOptions, args: &mut Vec<String>) {
        args.push(match opts.mode {
            GrepMode::Fixed => "--fixed-strings".to_string(),
            GrepMode::Basic => "--basic-regexp".to_string(),
            GrepMode::Extended => "--extended-regexp".to_string(),
            GrepMode::Pcre => "--perl-regexp".to_string(),
        });

        if let Some(c) = opts.context {
            args.push(format!("--context={}", c));
        }
        if let Some(b) = opts.before {
            args.push(format!("--before-context={}", b));
        }
        if let Some(a) = opts.after {
            args.push(format!("--after-context={}", a));
        }
        if opts.function {
            args.push("--function-context".to_string());
        }
        if opts.invert {
            args.push("--invert-match".to_string());
        }
        if opts.ignore_case {
            args.push("--ignore-case".to_string());
        }
        if opts.word_boundary {
            args.push("--word-regexp".to_string());
        }
        if let Some(depth) = opts.max_depth {
            args.push(format!("--max-depth={}", depth));
        }
    }

    pub fn grep(&self, pattern: &str, opts: &GrepOptions) -> Result<String> {
        // compose arguments
        let mut args = vec![
            "grep".to_string(),
            "--color=never".to_string(),
            "--line-number".to_string(),
            "-I".to_string(),     // exclude binary files
            "--null".to_string(), // for unambiguous delimiters
        ];

        self.expand_options(opts, &mut args);
        args.push(pattern.to_string());

        if !opts.only.is_empty() || !opts.exclude.is_empty() {
            args.push("--".to_string());
        }

        // append pathspec if "--only" exists
        if !opts.only.is_empty() {
            let only = opts
                .only
                .iter()
                .flat_map(|x| x.split(',').collect::<Vec<_>>());
            for pattern in only {
                args.push(pattern.to_string());
            }
        }

        // append pathspec if "--exclude" exists
        if !opts.exclude.is_empty() {
            let exclude = opts
                .exclude
                .iter()
                .flat_map(|x| x.split(',').collect::<Vec<_>>());
            for pattern in exclude {
                args.push(format!(":!{}", pattern));
            }
        }

        // run git-grep then parse the output as a utf-8 string
        let output = Command::new("git")
            .args(&args)
            .output()
            .context("failed to get output of \"git grep\". aborting.")?;
        let output = String::from_utf8(output.stdout).context(
            "failed to interpret the output of \"git grep\" as a UTF-8 string. aborting.",
        )?;

        Ok(output)
    }

    pub fn apply(&self, patch: &str) -> Result<()> {
        let mut apply = Command::new("git")
            .args(&["apply", "--unidiff-zero", "-"])
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

#[cfg(test)]
mod tests {
    use crate::{Git, GrepOptions};
    use clap::Parser;

    #[test]
    fn test_new() {
        assert!(Git::new().is_ok());
    }

    #[test]
    fn test_grep() {
        macro_rules! opts {
            ( $args: expr ) => {
                &GrepOptions::parse_from($args.split_whitespace())
            };
        }

        // assume tests/quick.txt exists
        let git = Git::new().unwrap();

        // "xyz" is a placeholder for a command name
        let output = git.grep("fox", opts!("xyz")).unwrap();
        assert!(output.lines().count() >= 2);

        let output = git.grep("fox", opts!("xyz -y tests/*.txt")).unwrap();
        assert!(output.lines().count() == 2);

        let output = git.grep("fox", opts!("xyz -x tests/*.txt -x src")).unwrap();
        assert!(output.lines().count() == 0);

        let output = git.grep("fox", opts!("xyz --max-depth 0")).unwrap();
        assert!(output.lines().count() == 0);

        let output = git.grep("fox", opts!("xyz --max-depth 1")).unwrap();
        assert!(output.lines().count() >= 2);

        let output = git.grep("FOX", opts!("xyz -y tests/*.txt -i")).unwrap();
        assert!(output.lines().count() == 2);

        let output = git.grep("quic", opts!("xyz -y tests/*.txt")).unwrap();
        assert!(output.lines().count() == 1);

        let output = git.grep("quic", opts!("xyz -y tests/*.txt -w")).unwrap();
        assert!(output.lines().count() == 0);

        let output = git.grep("fox", opts!("xyz -y tests/*.txt -v")).unwrap();
        assert!(output.lines().count() == 13);

        let output = git.grep("fox", opts!("xyz -y tests/*.txt -C2")).unwrap();
        assert!(output.lines().count() == 2 * 5 + 1);

        let output = git.grep("fox", opts!("xyz -y tests/*.txt -A1")).unwrap();
        assert!(output.lines().count() == 2 * 2 + 1);

        let output = git.grep("fox", opts!("xyz -y tests/*.txt -B1")).unwrap();
        assert!(output.lines().count() == 2 * 2 + 1);

        // TODO: --mode and --function-context
    }

    // TODO: git.apply
}
